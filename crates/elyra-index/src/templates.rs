//! Templates: Vue and Svelte single-file components and Laravel Blade views,
//! where much of a frontend or Laravel project's wiring is.
//!
//! - Vue and Svelte: the `<script>` sections are TypeScript or JavaScript.
//!   They are parsed in place, with everything else blanked (newlines kept),
//!   so lines and byte offsets are the file's own ([`script`]); the file
//!   defines its component, and the markup refers to components
//!   (`<OrderTable>`, `<order-table>`) and calls (`total()`, `@click="save"`).
//! - Blade: the file defines its view (`orders.row`) and, under
//!   `components/`, its component; it refers to components (`<x-order-table>`,
//!   `<livewire:cart>`, `@livewire('cart')`), views (`@include('orders.row')`)
//!   and methods (`$order->total()`, `Order::find()`).
//!
//! Without grammars of their own, so nothing new to keep in step with the
//! other tree-sitter crates.

use crate::{Definition, Extraction, Language, MAX_SIGNATURE_CHARS, MAX_TEXT_CHARS, Reference};
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Template {
    Vue,
    Svelte,
    Blade,
}

impl Template {
    pub fn from_path(path: &Path) -> Option<Template> {
        let name = path.file_name()?.to_str()?.to_ascii_lowercase();
        if name.ends_with(".blade.php") {
            Some(Template::Blade)
        } else if name.ends_with(".vue") {
            Some(Template::Vue)
        } else if name.ends_with(".svelte") {
            Some(Template::Svelte)
        } else {
            None
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Template::Vue => "vue",
            Template::Svelte => "svelte",
            Template::Blade => "blade",
        }
    }
}

/// A `<tag …>…</tag>` block: where its content starts and ends, and its
/// attributes.
struct Block {
    open: usize,
    content: usize,
    end: usize,
    close: usize,
    attributes: String,
}

/// The `<script>` or `<style>` blocks of a component.
fn blocks(source: &str, tag: &str) -> Vec<Block> {
    let lower = source.to_ascii_lowercase();
    let (opening, closing) = (format!("<{tag}"), format!("</{tag}"));
    let mut found = Vec::new();
    let mut at = 0;
    while let Some(start) = lower[at..].find(&opening).map(|i| at + i) {
        let after = start + opening.len();
        // `<scripts>` isn't `<script>`.
        if !lower[after..].starts_with(|c: char| c == '>' || c.is_whitespace()) {
            at = after;
            continue;
        }
        let Some(content) = tag_end(&lower, after) else {
            break;
        };
        let end = lower[content..]
            .find(&closing)
            .map(|i| content + i)
            .unwrap_or(source.len());
        let close = lower[end..]
            .find('>')
            .map(|i| end + i + 1)
            .unwrap_or(source.len());
        found.push(Block {
            open: start,
            content,
            end,
            close,
            attributes: source[after..content - 1].to_string(),
        });
        at = close;
    }
    found
}

/// Where the tag whose attributes start at `from` ends (just past its `>`):
/// a `>` inside a quoted attribute doesn't end it (Vue's
/// `generic="T extends Record<string, unknown>"`).
fn tag_end(source: &str, from: usize) -> Option<usize> {
    let mut quote = None;
    for (i, byte) in source.as_bytes()[from..].iter().enumerate() {
        match (quote, byte) {
            (None, b'"' | b'\'') => quote = Some(*byte),
            (Some(q), _) if q == *byte => quote = None,
            (None, b'>') => return Some(from + i + 1),
            _ => {}
        }
    }
    None
}

/// `source` with the bytes outside `keep` turned into spaces (newlines
/// stay): same length, same lines.
fn blank_outside(source: &str, keep: &[(usize, usize)]) -> String {
    let mut bytes = source.as_bytes().to_vec();
    for (index, byte) in bytes.iter_mut().enumerate() {
        if *byte != b'\n' && !keep.iter().any(|(from, to)| (*from..*to).contains(&index)) {
            *byte = b' ';
        }
    }
    String::from_utf8(bytes).unwrap_or_default()
}

/// A Vue or Svelte file's code as one grammar parses it: its `<script>`
/// sections where they are, the rest blank. None for Blade, or without a
/// script.
pub fn script(template: Template, source: &str) -> Option<(Language, String)> {
    if template == Template::Blade {
        return None;
    }
    let scripts = blocks(source, "script");
    if scripts.is_empty() {
        return None;
    }
    let language = scripts
        .iter()
        .map(|block| {
            let attributes = block.attributes.to_ascii_lowercase().replace('\'', "\"");
            if attributes.contains("lang=\"tsx\"") {
                Language::Tsx
            } else if attributes.contains("lang=\"ts\"")
                || attributes.contains("lang=\"typescript\"")
            {
                Language::TypeScript
            } else {
                Language::JavaScript
            }
        })
        .max_by_key(|language| match language {
            Language::Tsx => 2,
            Language::TypeScript => 1,
            _ => 0,
        })
        .unwrap_or(Language::JavaScript);
    let keep: Vec<(usize, usize)> = scripts.iter().map(|b| (b.content, b.end)).collect();
    Some((language, blank_outside(source, &keep)))
}

/// Definitions and references of a template file (`rel`: its path in the
/// project).
pub fn extract(template: Template, rel: &str, source: &str) -> Extraction {
    let mut extraction = match template {
        Template::Blade => Extraction::default(),
        _ => script(template, source)
            .and_then(|(language, code)| crate::extract(language, &code).ok())
            .unwrap_or_default(),
    };
    let lines: Vec<&str> = source.split('\n').collect();
    let whole = |name: String, kind: &str| Definition {
        name,
        kind: kind.to_string(),
        line: 1,
        col: 0,
        end_line: lines.len().max(1) as u32,
        container: None,
        signature: lines
            .iter()
            .find(|l| !l.trim().is_empty())
            .map(|l| crate::truncate_chars(l.trim(), MAX_SIGNATURE_CHARS))
            .unwrap_or_default(),
        start_byte: 0,
        end_byte: source.len(),
    };
    let stem = file_stem(rel);
    match template {
        Template::Blade => {
            if let Some(view) = view_name(rel) {
                extraction.definitions.insert(0, whole(view, "view"));
            }
            if let Some(component) = blade_component(rel) {
                extraction
                    .definitions
                    .insert(0, whole(component, "component"));
            }
        }
        _ => {
            if stem.starts_with(|c: char| c.is_ascii_alphabetic()) {
                extraction
                    .definitions
                    .insert(0, whole(studly(&stem), "component"));
            }
        }
    }
    // What the markup uses.
    let markup = match template {
        Template::Blade => source.to_string(),
        _ => {
            let mut keep: Vec<(usize, usize)> = vec![(0, source.len())];
            for block in blocks(source, "script")
                .into_iter()
                .chain(blocks(source, "style"))
            {
                keep = keep
                    .into_iter()
                    .flat_map(|(from, to)| {
                        [
                            (from, block.open.clamp(from, to)),
                            (block.close.clamp(from, to), to),
                        ]
                    })
                    .filter(|(from, to)| from < to)
                    .collect();
            }
            blank_outside(source, &keep)
        }
    };
    let mut found = Vec::new();
    match template {
        Template::Blade => blade_references(&markup, &mut found),
        _ => markup_references(template, &markup, &mut found),
    }
    let mut seen = std::collections::HashSet::new();
    for (offset, name, kind) in found {
        if name.is_empty() || !seen.insert((offset, name.clone())) {
            continue;
        }
        let line = source[..offset].matches('\n').count();
        let col = offset - source[..offset].rfind('\n').map(|i| i + 1).unwrap_or(0);
        extraction.references.push(Reference {
            name,
            kind: kind.to_string(),
            line: line as u32 + 1,
            col: col as u32,
            text: lines
                .get(line)
                .map(|l| crate::truncate_chars(l.trim(), MAX_TEXT_CHARS))
                .unwrap_or_default(),
        });
    }
    extraction.references.sort_by_key(|r| (r.line, r.col));
    extraction
}

fn file_stem(rel: &str) -> String {
    let name = rel.rsplit('/').next().unwrap_or(rel);
    name.split('.').next().unwrap_or(name).to_string()
}

/// `order-table`, `order_table` → `OrderTable`.
pub fn studly(name: &str) -> String {
    name.split(['-', '_', ' '])
        .filter(|part| !part.is_empty())
        .map(|part| {
            let mut chars = part.chars();
            chars
                .next()
                .map(|first| first.to_uppercase().chain(chars).collect::<String>())
                .unwrap_or_default()
        })
        .collect()
}

/// `resources/views/orders/row.blade.php` → `orders.row`.
fn view_name(rel: &str) -> Option<String> {
    let at = rel.rfind("views/")? + "views/".len();
    let view = rel[at..].strip_suffix(".blade.php")?;
    Some(view.replace('/', "."))
}

/// `…/views/components/order/line-item.blade.php` → `LineItem` (what
/// `<x-order.line-item>` names).
fn blade_component(rel: &str) -> Option<String> {
    let at = rel.rfind("views/components/")? + "views/components/".len();
    let path = rel[at..].strip_suffix(".blade.php")?;
    let last = path.rsplit('/').next()?;
    let last = if last == "index" {
        path.rsplit('/').nth(1)?
    } else {
        last
    };
    Some(studly(last))
}

fn is_name(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

/// The name that starts at `at`, if any.
fn name_at(text: &str, at: usize) -> &str {
    let rest = &text[at..];
    let end = rest.find(|c: char| !is_name(c)).unwrap_or(rest.len());
    &rest[..end]
}

/// A tag's name: `x-order.line-item`, `livewire:cart`, `OrderTable`.
fn tag_at(text: &str, at: usize) -> &str {
    let rest = &text[at..];
    let end = rest
        .find(|c: char| !(is_name(c) || matches!(c, '-' | '.' | ':')))
        .unwrap_or(rest.len());
    &rest[..end]
}

const NOT_CALLS: &[&str] = &[
    "if", "for", "while", "switch", "return", "function", "catch", "isset", "empty", "foreach",
    "elseif", "unless", "each", "include", "extends", "section", "yield", "push", "props", "await",
    "typeof", "new", "in", "of", "and", "or", "not",
];

/// `name(` calls in a stretch of markup (`{{ total() }}`, `$order->total()`).
fn calls(text: &str, found: &mut Vec<(usize, String, &'static str)>) {
    for (index, _) in text.match_indices('(') {
        let before = &text[..index];
        let start = before
            .rfind(|c: char| !is_name(c))
            .map(|i| i + 1)
            .unwrap_or(0);
        let name = &before[start..];
        let preceded = before[..start].chars().next_back();
        // Not a directive (`@if(`) or a variable (`$fn(`).
        if name.is_empty()
            || name.starts_with(|c: char| c.is_ascii_digit())
            || matches!(preceded, Some('@' | '$'))
            || NOT_CALLS.contains(&name)
        {
            continue;
        }
        found.push((start, name.to_string(), "call"));
    }
}

fn markup_references(
    template: Template,
    markup: &str,
    found: &mut Vec<(usize, String, &'static str)>,
) {
    for (index, _) in markup.match_indices('<') {
        let tag = tag_at(markup, index + 1);
        if tag.contains(':') {
            continue;
        }
        let name = if tag.starts_with(|c: char| c.is_ascii_uppercase()) {
            tag.rsplit('.').next().unwrap_or(tag).to_string()
        } else if tag.contains('-') {
            studly(tag)
        } else {
            continue;
        };
        found.push((index + 1, name, "component"));
    }
    // Handlers named without a call: `@click="save"`, `v-on:submit="send"`,
    // `on:click={save}`.
    let (prefixes, quote): (&[&str], &[char]) = match template {
        Template::Vue => (&["@", "v-on:"], &['"', '\'']),
        _ => (&["on:"], &['{']),
    };
    for prefix in prefixes {
        for (index, _) in markup.match_indices(prefix) {
            let rest = &markup[index + prefix.len()..];
            let Some(equals) = rest.find('=') else {
                continue;
            };
            if !rest[..equals]
                .chars()
                .all(|c| is_name(c) || matches!(c, '.' | '-'))
            {
                continue;
            }
            let value_at = index + prefix.len() + equals + 1;
            if !markup[value_at..].starts_with(quote) {
                continue;
            }
            let name = name_at(markup, value_at + 1);
            let after = markup[value_at + 1 + name.len()..].chars().next();
            if !name.is_empty() && after.is_some_and(|c| quote.contains(&c) || c == '}' || c == '"')
            {
                found.push((value_at + 1, name.to_string(), "call"));
            }
        }
    }
    // Calls inside expressions: `{{ … }}` (Vue), `{ … }` and attribute values.
    calls(markup, found);
}

fn blade_references(source: &str, found: &mut Vec<(usize, String, &'static str)>) {
    for (index, _) in source.match_indices('<') {
        let tag = tag_at(source, index + 1);
        let component = if let Some(name) = tag.strip_prefix("x-") {
            (!matches!(name, "slot" | "dynamic-component") && !name.starts_with("slot:"))
                .then(|| name.rsplit(['.', ':']).next().unwrap_or(name))
        } else {
            tag.strip_prefix("livewire:")
                .map(|name| name.rsplit('.').next().unwrap_or(name))
        };
        if let Some(name) = component.filter(|n| !n.is_empty()) {
            found.push((index + 1, studly(name), "component"));
        }
    }
    for directive in [
        "@livewire(",
        "@include(",
        "@includeIf(",
        "@includeWhen(",
        "@includeUnless(",
        "@extends(",
        "@each(",
        "@component(",
    ] {
        for (index, _) in source.match_indices(directive) {
            let args_at = index + directive.len();
            let args = &source[args_at..];
            let close = args.find(')').unwrap_or(args.len());
            // The view is the first string (a condition before it isn't one).
            let mut strings = Vec::new();
            let mut rest = &args[..close];
            let mut offset = args_at;
            while let Some(open) = rest.find(['\'', '"']) {
                let quote = rest.as_bytes()[open] as char;
                let Some(end) = rest[open + 1..].find(quote) else {
                    break;
                };
                strings.push((offset + open + 1, &rest[open + 1..open + 1 + end]));
                offset += open + end + 2;
                rest = &rest[open + end + 2..];
            }
            let Some((at, value)) = strings.first().copied() else {
                // `@livewire(CartSummary::class)`.
                if directive == "@livewire(" {
                    let start = args_at + (close - args[..close].trim_start().len());
                    let name = name_at(source, start);
                    if !name.is_empty() {
                        found.push((start, name.to_string(), "component"));
                    }
                }
                continue;
            };
            if directive == "@livewire(" {
                found.push((
                    at,
                    studly(value.rsplit('.').next().unwrap_or(value)),
                    "component",
                ));
            } else {
                found.push((at, value.to_string(), "view"));
            }
        }
    }
    // Method calls: `$order->total()`, `Order::find()`.
    for arrow in ["->", "::"] {
        for (index, _) in source.match_indices(arrow) {
            let at = index + arrow.len();
            let name = name_at(source, at);
            if !name.is_empty() && name != "class" && source[at + name.len()..].starts_with('(') {
                found.push((at, name.to_string(), "call"));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Template, extract, script, studly};
    use std::path::Path;

    const VUE: &str = r#"<template>
  <div>
    <OrderTable :rows="orders" @select="pick" />
    <order-summary v-if="ready" />
    <button @click="save">{{ formatTotal(total) }}</button>
  </div>
</template>

<script setup lang="ts">
import OrderTable from './OrderTable.vue'
function pick(order: Order) { selected.value = order }
function save() { api.store(selected.value) }
</script>

<style scoped>
.price { color: red }
</style>
"#;

    fn refs(extraction: &crate::Extraction) -> Vec<(String, &str, u32)> {
        extraction
            .references
            .iter()
            .map(|r| (r.name.clone(), r.kind.as_str(), r.line))
            .collect()
    }

    #[test]
    fn knows_templates_by_name() {
        assert_eq!(
            Template::from_path(Path::new("resources/views/orders/row.blade.php")),
            Some(Template::Blade)
        );
        assert_eq!(
            Template::from_path(Path::new("src/OrderTable.vue")),
            Some(Template::Vue)
        );
        assert_eq!(
            Template::from_path(Path::new("src/routes/+page.svelte")),
            Some(Template::Svelte)
        );
        assert_eq!(Template::from_path(Path::new("app/Order.php")), None);
        assert_eq!(studly("order-line_item"), "OrderLineItem");
    }

    #[test]
    fn a_generic_component_s_script_starts_after_its_tag() {
        // Vue 3.3: a `>` inside the `generic` attribute doesn't close the tag.
        let vue = "<script setup lang=\"ts\" generic=\"T extends Record<string, unknown>, K extends keyof T\">\nconst props = defineProps<{ rows: T[] }>()\nfunction sortBy(key: K) {}\n</script>\n\n<template><div /></template>\n";
        let (language, code) = script(Template::Vue, vue).unwrap();
        assert_eq!(language, crate::Language::TypeScript);
        assert_eq!(
            code.lines().next().unwrap().trim(),
            "",
            "the tag isn't code"
        );
        assert!(code.contains("function sortBy(key: K)"));
        let found = extract(Template::Vue, "src/DataTable.vue", vue);
        let definitions: Vec<(&str, u32)> = found
            .definitions
            .iter()
            .map(|d| (d.name.as_str(), d.line))
            .collect();
        assert!(definitions.contains(&("sortBy", 3)), "{definitions:?}");
    }

    #[test]
    fn reads_a_vue_component() {
        let (language, code) = script(Template::Vue, VUE).unwrap();
        assert_eq!(language, crate::Language::TypeScript);
        assert_eq!(code.len(), VUE.len(), "same bytes");
        assert_eq!(code.lines().count(), VUE.lines().count(), "same lines");
        assert!(!code.contains("<OrderTable") && code.contains("function save()"));
        let found = extract(Template::Vue, "src/components/OrderPanel.vue", VUE);
        let definitions: Vec<(&str, &str, u32)> = found
            .definitions
            .iter()
            .map(|d| (d.name.as_str(), d.kind.as_str(), d.line))
            .collect();
        assert_eq!(
            definitions,
            [
                ("OrderPanel", "component", 1),
                ("pick", "function", 11),
                ("save", "function", 12)
            ]
        );
        let references = refs(&found);
        for wanted in [
            ("OrderTable".to_string(), "component", 3),
            ("pick".to_string(), "call", 3),
            ("OrderSummary".to_string(), "component", 4),
            ("save".to_string(), "call", 5),
            ("formatTotal".to_string(), "call", 5),
            ("store".to_string(), "call", 12),
        ] {
            assert!(references.contains(&wanted), "{wanted:?} in {references:?}");
        }
        assert!(
            !references
                .iter()
                .any(|(name, _, line)| name == "div" || *line >= 15),
            "no HTML or CSS"
        );
    }

    #[test]
    fn reads_a_svelte_component() {
        let svelte = "<script>\n  export let total;\n  function checkout() {}\n</script>\n\n<Cart {total} on:click={checkout} />\n<p>{format(total)}</p>\n";
        let found = extract(Template::Svelte, "src/lib/CheckoutBox.svelte", svelte);
        assert_eq!(found.definitions[0].name, "CheckoutBox");
        assert!(
            found
                .definitions
                .iter()
                .any(|d| d.name == "checkout" && d.line == 3)
        );
        let references = refs(&found);
        assert!(
            references.contains(&("Cart".into(), "component", 6)),
            "{references:?}"
        );
        assert!(
            references.contains(&("checkout".into(), "call", 6)),
            "{references:?}"
        );
        assert!(
            references.contains(&("format".into(), "call", 7)),
            "{references:?}"
        );
        // SvelteKit's `+page.svelte` names no component.
        assert!(
            extract(Template::Svelte, "src/routes/+page.svelte", svelte).definitions[0].name
                != "+page"
        );
    }

    #[test]
    fn reads_a_blade_view() {
        let blade = "@extends('layouts.app')\n\n@section('content')\n  <x-order.line-item :order=\"$order\" />\n  <x-slot:footer>{{ $order->total() }}</x-slot:footer>\n  <livewire:cart-summary />\n  @livewire('admin.users-table')\n  @includeWhen($order->paid, 'orders.receipt')\n  @include('orders.row', ['order' => $order])\n  {{ Order::find($id)?->number }}\n  @if(count($orders))\n@endsection\n";
        let found = extract(
            Template::Blade,
            "resources/views/orders/show.blade.php",
            blade,
        );
        assert_eq!(found.definitions.len(), 1);
        assert_eq!(
            (
                found.definitions[0].name.as_str(),
                found.definitions[0].kind.as_str()
            ),
            ("orders.show", "view")
        );
        let references = refs(&found);
        for wanted in [
            ("layouts.app".to_string(), "view", 1),
            ("LineItem".to_string(), "component", 4),
            ("total".to_string(), "call", 5),
            ("CartSummary".to_string(), "component", 6),
            ("UsersTable".to_string(), "component", 7),
            ("orders.receipt".to_string(), "view", 8),
            ("orders.row".to_string(), "view", 9),
            ("find".to_string(), "call", 10),
        ] {
            assert!(references.contains(&wanted), "{wanted:?} in {references:?}");
        }
        assert!(
            !references
                .iter()
                .any(|(name, _, _)| name == "Slot" || name == "count" || name == "if"),
            "{references:?}"
        );
        let component = extract(
            Template::Blade,
            "resources/views/components/order/line-item.blade.php",
            "<div>{{ $slot }}</div>\n",
        );
        let names: Vec<&str> = component
            .definitions
            .iter()
            .map(|d| d.name.as_str())
            .collect();
        assert_eq!(names, ["LineItem", "components.order.line-item"]);
    }
}
