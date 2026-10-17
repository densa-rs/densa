use font8x8::legacy::BASIC_LEGACY;

/// Glyph cell size in atlas texels.
pub const GLYPH_PX: u32 = 8;
const ATLAS_COLS: u32 = 16;
const ATLAS_ROWS: u32 = 8;
pub const ATLAS_WIDTH: u32 = ATLAS_COLS * GLYPH_PX;
pub const ATLAS_HEIGHT: u32 = ATLAS_ROWS * GLYPH_PX;

/// 8-bit coverage atlas of the 128 ASCII glyphs of the public-domain font8x8.
///
/// PoC-grade text: crisp at integer scales, ASCII only. A shaped, proportional font with a
/// glyph cache (allocating only on cache misses) replaces it later.
#[derive(Debug, Clone)]
pub struct GlyphAtlas {
    pixels: Box<[u8]>,
}

impl GlyphAtlas {
    pub fn font8x8() -> Self {
        let mut pixels = vec![0u8; (ATLAS_WIDTH * ATLAS_HEIGHT) as usize];
        for (code, rows) in (0u32..).zip(BASIC_LEGACY.iter()) {
            let cell_x = (code % ATLAS_COLS) * GLYPH_PX;
            let cell_y = (code / ATLAS_COLS) * GLYPH_PX;
            for (y, bits) in (0u32..).zip(rows.iter()) {
                for x in 0..GLYPH_PX {
                    // font8x8 stores each row with bit 0 as the leftmost pixel.
                    if (bits >> x) & 1 == 1 {
                        let index = (cell_y + y) * ATLAS_WIDTH + cell_x + x;
                        pixels[index as usize] = 255;
                    }
                }
            }
        }
        Self {
            pixels: pixels.into_boxed_slice(),
        }
    }

    /// Row-major R8 texels, `ATLAS_WIDTH` bytes per row.
    pub fn pixels(&self) -> &[u8] {
        &self.pixels
    }

    pub fn pixel(&self, x: u32, y: u32) -> u8 {
        self.pixels[(y * ATLAS_WIDTH + x) as usize]
    }

    /// Normalized `[u0, v0, u1, v1]` of `ch`'s cell; non-ASCII bytes map to `?`.
    pub fn uv(&self, ch: u8) -> [f32; 4] {
        let code = u32::from(if ch.is_ascii() { ch } else { b'?' });
        let x = ((code % ATLAS_COLS) * GLYPH_PX) as f32;
        let y = ((code / ATLAS_COLS) * GLYPH_PX) as f32;
        let (w, h) = (ATLAS_WIDTH as f32, ATLAS_HEIGHT as f32);
        let g = GLYPH_PX as f32;
        [x / w, y / h, (x + g) / w, (y + g) / h]
    }
}
