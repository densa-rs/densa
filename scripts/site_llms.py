"""Writes densa.dev's llms.txt, llms-full.txt and AGENTS.md from this repository: AGENTS.md,
each crate's Cargo.toml description and `//!` front page, the facade's task table and the
examples' first paragraphs.

usage: python scripts/site_llms.py SITE_FOLDER   (a checkout of densa.dev)

The files follow main: run it after the docs change, and at each release (the version and the
minimum Rust version come from the workspace's Cargo.toml).
"""
from __future__ import annotations

import pathlib
import re
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent
RAW = 'https://raw.githubusercontent.com/densa-rs/densa/main'

# Crates, in the order agents should read them.
CRATES = [
    'densa', 'densa-data', 'densa-arrow', 'densa-ui', 'densa-widgets', 'densa-window',
    'densa-chart', 'densa-grid', 'densa-draw', 'densa-render', 'densa-paragraph', 'densa-text',
    'densa-core', 'densa-fmt', 'densa-sync', 'densa-platform', 'densa-alloc',
]

# Examples worth copying, in order: (crate, file).
EXAMPLES = [
    ('densa-widgets', 'monitor.rs'), ('densa-widgets', 'linked.rs'),
    ('densa-widgets', 'live_table.rs'), ('densa-widgets', 'paged.rs'),
    ('densa-widgets', 'dock.rs'), ('densa-widgets', 'gallery.rs'),
    ('densa-widgets', 'chart.rs'), ('densa-widgets', 'dense.rs'),
    ('densa-arrow', 'table.rs'), ('densa-paragraph', 'editor.rs'),
]

SUMMARY = (
    'densa is an open-source Rust platform for applications that show a lot of live data: '
    'live Arrow tables kept sorted, filtered and aggregated as they change, a GPU interface '
    'with a data grid, charts, docking and screen reader support, and a runtime that hands '
    'data between threads without waiting. Version {version} on crates.io; Rust {rust} or later.'
)

INTRO = (
    'Read AGENTS.md first: the versions to use (densa is newer than most training data), how '
    'the pieces fit, and which example to copy. These files follow densa\'s main branch: what '
    'the CHANGELOG lists under Unreleased is not on crates.io yet.'
)

BEFORE = [
    'Fetch these files as they are (for example with curl): a tool that summarizes a page drops '
    'the item names and signatures. llms-full.txt has every crate\'s front page, with an example '
    'each, in one file.',
    'The source cargo built, with every doc comment and example, is on your disk: from git, '
    '~/.cargo/git/checkouts/densa-*/<commit>/ where <commit> is the first 7 characters of the '
    'hash after # in the densa entries of your Cargo.lock (the other folders there are older '
    'commits); from crates.io, ~/.cargo/registry/src/*/densa-<crate>-<version>/. The list below '
    'says which file documents each item.',
    'Build record batches with the Arrow types densa re-exports: densa::data::arrow_array and '
    'densa::data::arrow_schema.',
    'Widgets change only on the window thread, through ui.update; other threads send data and '
    'call Waker::wake (get the Waker from host.waker() in App::build).',
]

START = [
    '[AGENTS.md](https://densa.dev/AGENTS.md): versions, how the pieces fit, examples to copy, '
    'testing without a window, rules',
    '[densa on docs.rs](https://docs.rs/densa): which module for which task, and a complete '
    'program',
    f'[README]({RAW}/README.md): overview and usage',
    f'[CHANGELOG]({RAW}/CHANGELOG.md): what each release changed',
]

OPTIONAL = [
    '[llms-full.txt](https://densa.dev/llms-full.txt): AGENTS.md and every crate\'s front page '
    'in one file',
    '[densa-connectors](https://github.com/densa-rs/densa-connectors): CSV and Parquet files, '
    'WebSocket streams, Arrow Flight and SQL with DataFusion, and Parquet files, DataFusion or '
    'Flight SQL queries a page at a time for the paged grid (a file sorted by any column, by a '
    'sorted copy made once)',
    '[densa Studio](https://densa.dev/studio/): an application built on densa: files, streams '
    'and Flight in grids, SQL and charts (source: https://github.com/densa-rs/densa-studio)',
]


def read(path: pathlib.Path) -> str:
    return path.read_text(encoding='utf-8').replace('\r\n', '\n')


def toml_value(text: str, key: str, where: str) -> str:
    """The value of a `key = "value"` line of `text`, or an exit naming `where`."""
    found = re.search(rf'^{key}\s*=\s*"([^"]+)"', text, re.M)
    if found is None:
        sys.exit(f'no {key} = "..." in {where}')
    return found.group(1)


def workspace_value(key: str) -> str:
    """A `key = "value"` of the workspace's [workspace.package]."""
    text = read(ROOT / 'Cargo.toml')
    start = text.find('[workspace.package]')
    if start < 0:
        sys.exit('no [workspace.package] in Cargo.toml')
    return toml_value(text[start:], key, 'Cargo.toml [workspace.package]')


def description(crate: str) -> str:
    text = read(ROOT / 'crates' / crate / 'Cargo.toml')
    return toml_value(text, 'description', f'crates/{crate}/Cargo.toml')


def doc_lines(path: pathlib.Path) -> list[str]:
    """The `//!` lines at the top of `path`, without their markers."""
    lines = []
    for line in read(path).split('\n'):
        if line.startswith('//!'):
            lines.append(line[4:] if line.startswith('//! ') else line[3:])
        elif lines:
            break
    return lines


def plain_code(lines: list[str]) -> list[str]:
    """`lines` of rustdoc as Markdown: code blocks marked rust, lines rustdoc hides dropped."""
    out, code = [], False
    for line in lines:
        if line.startswith('```'):
            if not code:
                out.append('```rust' if line.strip('`').strip() in ('', 'rust', 'no_run',
                                                                  'ignore', 'should_panic',
                                                                  'compile_fail') or
                           line.strip('`').startswith(('no_run', 'rust,', 'ignore'))
                           else line)
            else:
                out.append('```')
            code = not code
            continue
        if code and (line == '#' or line.startswith('# ')):
            continue
        out.append(line)
    return out


def first_paragraph(lines: list[str]) -> str:
    paragraph = []
    for line in lines:
        if not line.strip():
            if paragraph:
                break
            continue
        paragraph.append(line.strip())
    return ' '.join(paragraph)


# Where an item of a densa module is defined: crates/densa-<module>/src/....
DEFINES = re.compile(r'^\s*pub (?:struct|enum|trait|fn|type|const|static) (\w+)', re.M)


def defining_file(path: str) -> str | None:
    """The file defining `densa::module::Item[::member]`, or a module's lib.rs."""
    parts = path.split('::')
    if parts[0] != 'densa' or len(parts) < 2:
        return None
    crate = ROOT / 'crates' / f'densa-{parts[1]}' / 'src'
    if not crate.is_dir():
        return None
    if len(parts) == 2:
        return f'crates/densa-{parts[1]}/src/lib.rs'
    name = parts[2].rstrip('()')
    for file in sorted(crate.rglob('*.rs')):
        if name in DEFINES.findall(read(file)):
            return file.relative_to(ROOT).as_posix()
    return None


def tasks() -> list[str]:
    """The facade's "Which module for which task" rows, as llms.txt lines."""
    lines = doc_lines(ROOT / 'crates' / 'densa' / 'src' / 'lib.rs')
    rows = [line for line in lines if line.startswith('| ') and not line.startswith('| To ')]
    out = []
    for row in rows:
        task, use = [cell.strip() for cell in row.strip('|').split('|')]
        # Links to plain code, and module paths from the facade's root.
        use = re.sub(r'\[`([^`]+)`\]', r'`\1`', use)
        use = re.sub(r'`((?:[a-z_]+::)*[A-Za-z_][\w:]*\(?\)?)`',
                     lambda m: '`' + (m.group(1) if m.group(1).startswith('densa')
                                      else 'densa::' + m.group(1)) + '`', use)
        files = []
        for path in re.findall(r'`(densa::[\w:]+)', use):
            file = defining_file(path)
            if file and file not in files:
                files.append(file)
        where = f' (in {", ".join(files)})' if files else ''
        out.append(f'- To {task}: {use}{where}')
    return out


def llms_txt() -> str:
    version, rust = workspace_value('version'), workspace_value('rust-version')
    lines = ['# densa', '', '> ' + SUMMARY.format(version=version, rust=rust), '', INTRO, '',
             '## Before you start', '']
    lines += [f'- {item}' for item in BEFORE]
    lines += ['', '## Which item for which task', ''] + tasks()
    lines += ['', '## Start here', ''] + [f'- {item}' for item in START]
    lines += ['', '## Crates', '']
    for crate in CRATES:
        front = f'{RAW}/crates/{crate}/src/lib.rs'
        lines.append(f'- [{crate}](https://docs.rs/{crate}): {description(crate)} '
                     f'(front page on main: {front})')
    lines += ['', '## Examples', '']
    for crate, file in EXAMPLES:
        path = ROOT / 'crates' / crate / 'examples' / file
        about = first_paragraph(doc_lines(path))
        lines.append(f'- [{file}]({RAW}/crates/{crate}/examples/{file}): {about}')
    lines += ['', '## Optional', ''] + [f'- {item}' for item in OPTIONAL]
    return '\n'.join(lines) + '\n'


def llms_full_txt() -> str:
    version, rust = workspace_value('version'), workspace_value('rust-version')
    agents = read(ROOT / 'AGENTS.md').split('\n')
    agents[0] = '# AGENTS.md (from the densa repository)'
    parts = ['\n'.join(['# densa', '', '> ' + SUMMARY.format(version=version, rust=rust), '']
                       + agents).rstrip('\n')]
    for crate in CRATES:
        front = plain_code(doc_lines(ROOT / 'crates' / crate / 'src' / 'lib.rs'))
        parts.append('\n'.join([f'# Crate {crate} (https://docs.rs/{crate})', '',
                                description(crate), ''] + front).rstrip('\n'))
    return '\n\n---\n\n'.join(parts) + '\n'


def main() -> None:
    if len(sys.argv) != 2:
        sys.exit(__doc__)
    site = pathlib.Path(sys.argv[1])
    if not (site / 'index.html').exists():
        sys.exit(f'{site} is not a checkout of densa.dev (no index.html)')
    (site / 'llms.txt').write_text(llms_txt(), encoding='utf-8', newline='\n')
    (site / 'llms-full.txt').write_text(llms_full_txt(), encoding='utf-8', newline='\n')
    (site / 'AGENTS.md').write_text(read(ROOT / 'AGENTS.md'), encoding='utf-8', newline='\n')
    print(f'wrote llms.txt, llms-full.txt and AGENTS.md in {site}')


if __name__ == '__main__':
    main()
