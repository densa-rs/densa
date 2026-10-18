use std::borrow::Cow;

use densa_core::Color;
use densa_text::{Font, FontBytes, FontError};
use parley::fontique::{
    Blob, Collection, CollectionOptions, FallbackKey, FamilyId, GenericFamily, Language, Script,
    SourceCache,
};
use parley::{FontContext, FontData, FontFamily, LayoutContext, LineHeight, StyleProperty};
use rustc_hash::FxHashMap;

use crate::paragraph::{bytes_key, face_key, FaceKey, Paragraph};
use crate::style::{
    check_font_size, check_line_height, check_span, check_width, ParagraphError, ParagraphStyle,
    Span,
};

/// Generic families the bundled font stands in for when system fonts are off.
const GENERICS: [GenericFamily; 9] = [
    GenericFamily::Serif,
    GenericFamily::SansSerif,
    GenericFamily::Monospace,
    GenericFamily::SystemUi,
    GenericFamily::UiSerif,
    GenericFamily::UiSansSerif,
    GenericFamily::UiMonospace,
    GenericFamily::UiRounded,
    GenericFamily::Emoji,
];

/// Generic families that stay the bundled font with system fonts on, so turning them on only
/// adds characters: text the bundled font could draw looks the same.
const MONOSPACE: [GenericFamily; 2] = [GenericFamily::Monospace, GenericFamily::UiMonospace];

/// Han locales whose system fallback fonts are tried, in order, after the default one.
const HAN_LOCALES: [&str; 4] = ["zh-TW", "zh-HK", "ja", "ko"];

/// Simplified Chinese families tried last. fontique's Windows backend answers the default Han
/// key (which is simplified Chinese) with Yu Gothic UI, a Japanese font without many
/// simplified forms; these fill them in on Windows, macOS and Linux.
const SIMPLIFIED_CHINESE: [&str; 5] = [
    "Microsoft YaHei UI",
    "Microsoft YaHei",
    "PingFang SC",
    "Noto Sans CJK SC",
    "Source Han Sans SC",
];

/// Makes the default Han fallback a chain of every locale's font, so a Han character any
/// installed CJK font has is drawn: no single font has the characters of every locale, and
/// fontique gives each script and locale one. Parley also falls back to this chain for
/// punctuation in every script.
fn chain_han_fallbacks(collection: &mut Collection) {
    let han = Script::from_bytes(*b"Hani");
    let mut chain: Vec<FamilyId> = collection
        .fallback_families(FallbackKey::new(han, None))
        .collect();
    let add = |chain: &mut Vec<FamilyId>, family: FamilyId| {
        if !chain.contains(&family) {
            chain.push(family);
        }
    };
    for tag in HAN_LOCALES {
        let Ok(locale) = Language::parse(tag) else {
            continue;
        };
        let families: Vec<FamilyId> = collection
            .fallback_families(FallbackKey::new(han, Some(&locale)))
            .collect();
        for family in families {
            add(&mut chain, family);
        }
    }
    for name in SIMPLIFIED_CHINESE {
        if let Some(family) = collection.family_id(name) {
            add(&mut chain, family);
        }
    }
    collection.set_fallbacks(FallbackKey::new(han, None), chain.into_iter());
}

/// Fonts plus layout scratch space: builds [`Paragraph`]s.
///
/// Create one per application (or per UI thread) and keep it: it caches font data and reuses
/// layout buffers. Every face a paragraph uses becomes one `densa_text::Font`, so all
/// paragraphs (and the bundled font's other users, such as the grid) share cached glyphs.
/// Faces stay loaded for the system's lifetime (system CJK fonts can be tens of MB each).
pub struct TextSystem {
    fonts: FontContext,
    layouts: LayoutContext<Color>,
    /// Every face any paragraph used, kept (with its font file) for the system's lifetime.
    faces: FxHashMap<FaceKey, Font>,
}

impl std::fmt::Debug for TextSystem {
    // Hand-written: parley's contexts do not implement Debug.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TextSystem")
            .field("faces", &self.faces.len())
            .finish_non_exhaustive()
    }
}

impl TextSystem {
    /// The text system applications want: the bundled JetBrains Mono for `monospace` (the
    /// default family), and the operating system's fonts for other families and for every
    /// character the bundled font lacks (Korean, Japanese, Chinese, symbols, ...).
    ///
    /// Without the `system-fonts` feature (on by default) this is [`bundled`](Self::bundled).
    ///
    /// # Errors
    /// Only if the bundled font were corrupt.
    pub fn new() -> Result<Self, FontError> {
        Self::build(cfg!(feature = "system-fonts"))
    }

    /// A text system with only the bundled JetBrains Mono, which also stands in for every
    /// generic family (`monospace`, `sans-serif`, ...): the same text lays out the same on
    /// every machine, for tests and screenshots. Characters it lacks are drawn as its
    /// missing-glyph box; add fonts with [`register_font`](Self::register_font).
    ///
    /// # Errors
    /// Only if the bundled font were corrupt.
    pub fn bundled() -> Result<Self, FontError> {
        Self::build(false)
    }

    fn build(system_fonts: bool) -> Result<Self, FontError> {
        let collection = Collection::new(CollectionOptions {
            shared: false,
            system_fonts,
        });
        let mut fonts = FontContext {
            collection,
            source_cache: SourceCache::default(),
        };
        let bundled = Font::bundled_mono()?;
        let blob = Blob::new(bundled.shared_bytes());
        let registered = fonts.collection.register_fonts(blob, None);
        let Some(&(family, _)) = registered.first() else {
            return Err(FontError::Parse);
        };
        let generics: &[GenericFamily] = if system_fonts { &MONOSPACE } else { &GENERICS };
        for &generic in generics {
            fonts
                .collection
                .set_generic_families(generic, std::iter::once(family));
        }
        if system_fonts {
            chain_han_fallbacks(&mut fonts.collection);
        }
        let mut faces = FxHashMap::default();
        // The bundled font keeps its process-wide identity, shared with its other users.
        faces.insert(bytes_key(bundled.bytes(), 0), bundled);
        Ok(Self {
            fonts,
            layouts: LayoutContext::new(),
            faces,
        })
    }

    /// Adds the fonts in `data` (a font file or collection); returns their family names.
    ///
    /// # Errors
    /// [`FontError::Parse`] if `data` holds no readable font.
    pub fn register_font(&mut self, data: FontBytes) -> Result<Vec<String>, FontError> {
        let registered = self.fonts.collection.register_fonts(Blob::new(data), None);
        if registered.is_empty() {
            return Err(FontError::Parse);
        }
        let mut names = Vec::with_capacity(registered.len());
        for (family, _) in &registered {
            // `family_name` borrows the collection mutably: copy each name out.
            if let Some(name) = self.fonts.collection.family_name(*family) {
                names.push(name.to_owned());
            }
        }
        Ok(names)
    }

    /// Shapes `text` with `style`, `spans` overriding parts of it, then breaks and aligns it.
    ///
    /// Allocates (shaping is not a per-frame operation); drawing the result does not.
    ///
    /// # Errors
    /// [`ParagraphError`] for a size, line height or width out of range, a span that does not
    /// lie on character boundaries within `text`, or a font that cannot be read.
    pub fn paragraph(
        &mut self,
        text: &str,
        style: &ParagraphStyle,
        spans: &[Span],
    ) -> Result<Paragraph, ParagraphError> {
        check_font_size(style.size_px)?;
        check_line_height(style.line_height)?;
        let max_width = check_width(style.max_width)?;
        for span in spans {
            check_span(text, span)?;
        }

        let mut builder = self
            .layouts
            .ranged_builder(&mut self.fonts, text, 1.0, true);
        let family = FontFamily::Source(Cow::Borrowed(&style.family));
        builder.push_default(StyleProperty::FontFamily(family));
        builder.push_default(StyleProperty::FontSize(style.size_px));
        let line_height = LineHeight::FontSizeRelative(style.line_height);
        builder.push_default(StyleProperty::LineHeight(line_height));
        builder.push_default(StyleProperty::Brush(style.color));
        for span in spans {
            if let Some(color) = span.color {
                builder.push(StyleProperty::Brush(color), span.range.clone());
            }
            if let Some(size) = span.size_px {
                builder.push(StyleProperty::FontSize(size), span.range.clone());
            }
        }
        let layout = builder.build(text);

        let faces = &mut self.faces;
        let mut resolve = |data: &FontData| -> Option<Font> {
            let key = face_key(data);
            if let Some(font) = faces.get(&key) {
                return Some(font.clone());
            }
            let (bytes, _) = data.data.clone().into_raw_parts();
            let font = Font::from_shared(bytes, data.index).ok()?;
            faces.insert(key, font.clone());
            Some(font)
        };
        Paragraph::new(layout, text.len(), style.align, max_width, &mut resolve)
    }
}
