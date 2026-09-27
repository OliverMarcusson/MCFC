//! Compile-time MiniMessage: a string literal becomes one text component,
//! written as SNBT. Runtime strings use `std.text.parseMiniMessage`, which only
//! knows style tags.

pub const NAMED_COLORS: &[&str] = &[
    "black",
    "dark_blue",
    "dark_green",
    "dark_aqua",
    "dark_red",
    "dark_purple",
    "gold",
    "gray",
    "dark_gray",
    "blue",
    "green",
    "aqua",
    "red",
    "light_purple",
    "yellow",
    "white",
];

const NAMED_RGB: &[u32] = &[
    0x000000, 0x0000AA, 0x00AA00, 0x00AAAA, 0xAA0000, 0xAA00AA, 0xFFAA00, 0xAAAAAA, 0x555555,
    0x5555FF, 0x55FF55, 0x55FFFF, 0xFF5555, 0xFF55FF, 0xFFFF55, 0xFFFFFF,
];

type Style = Vec<(&'static str, String)>;

struct Open {
    name: String,
    style: Style,
    gradient: Option<usize>,
}

struct Run {
    /// A `text` run, or the raw content keys of a `keybind`/`translate` run.
    content: String,
    text: Option<String>,
    style: Style,
    gradient: Option<usize>,
}

enum Gradient {
    Colors(Vec<u32>),
    Rainbow,
}

pub fn to_snbt(source: &str) -> String {
    let mut stack: Vec<Open> = Vec::new();
    let mut runs: Vec<Run> = Vec::new();
    let mut gradients: Vec<Gradient> = Vec::new();
    let mut buf = String::new();
    let chars: Vec<char> = source.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '\\' && matches!(chars.get(i + 1), Some('<' | '\\')) {
            buf.push(chars[i + 1]);
            i += 2;
            continue;
        }
        let end = (c == '<').then(|| tag_end(&chars, i + 1)).flatten();
        let Some(end) = end else {
            buf.push(c);
            i += 1;
            continue;
        };
        let raw: String = chars[i + 1..end].iter().collect();
        flush(&mut buf, &stack, &mut runs);
        if apply_tag(&raw, &mut stack, &mut runs, &mut gradients).is_some() {
            i = end + 1;
        } else {
            // Not a tag: it's text, merged back into its neighbours below.
            buf.push('<');
            i += 1;
        }
    }
    flush(&mut buf, &stack, &mut runs);
    let mut merged: Vec<Run> = Vec::new();
    for run in expand_gradients(runs, &gradients) {
        match merged.last_mut() {
            Some(last) if last.style == run.style && last.text.is_some() && run.text.is_some() => {
                last.text
                    .as_mut()
                    .unwrap()
                    .push_str(run.text.as_ref().unwrap());
            }
            _ => merged.push(run),
        }
    }
    let parts: Vec<String> = merged.iter().map(render_run).collect();
    format!("{{text:\"\",extra:[{}]}}", parts.join(","))
}

fn tag_end(chars: &[char], from: usize) -> Option<usize> {
    let mut quote = None;
    let mut i = from;
    while i < chars.len() {
        match (quote, chars[i]) {
            (None, '\'' | '"') => quote = Some(chars[i]),
            (Some(q), c) if c == q => quote = None,
            (None, '>') => return Some(i),
            (None, '<') => return None,
            _ => {}
        }
        i += 1;
    }
    None
}

fn split_args(raw: &str) -> Vec<String> {
    let mut args = vec![String::new()];
    let mut quote = None;
    for c in raw.chars() {
        match (quote, c) {
            (None, '\'' | '"') => quote = Some(c),
            (Some(q), c) if c == q => quote = None,
            (None, ':') => args.push(String::new()),
            (_, c) => args.last_mut().unwrap().push(c),
        }
    }
    args
}

fn flush(buf: &mut String, stack: &[Open], runs: &mut Vec<Run>) {
    if buf.is_empty() {
        return;
    }
    runs.push(styled_run(Some(std::mem::take(buf)), String::new(), stack));
}

fn styled_run(text: Option<String>, content: String, stack: &[Open]) -> Run {
    let mut style: Style = Vec::new();
    for open in stack {
        for (key, value) in &open.style {
            style.retain(|(k, _)| k != key);
            style.push((key, value.clone()));
        }
    }
    Run {
        content,
        text,
        style,
        gradient: stack.iter().rev().find_map(|open| open.gradient),
    }
}

fn apply_tag(
    raw: &str,
    stack: &mut Vec<Open>,
    runs: &mut Vec<Run>,
    gradients: &mut Vec<Gradient>,
) -> Option<()> {
    if let Some(name) = raw.strip_prefix('/') {
        let name = canonical(&split_args(name)[0].to_lowercase());
        return match stack.iter().rposition(|open| open.name == name) {
            Some(index) => {
                stack.remove(index);
                Some(())
            }
            None => None,
        };
    }
    let args = split_args(raw);
    let tag = args[0].to_lowercase();
    let arg = |n: usize| args.get(n).cloned().unwrap_or_default();
    let mut style: Style = Vec::new();
    let mut gradient = None;
    match tag.as_str() {
        "reset" => {
            stack.clear();
            return Some(());
        }
        "newline" | "br" => {
            runs.push(styled_run(Some("\n".into()), String::new(), stack));
            return Some(());
        }
        "key" if args.len() == 2 => {
            let content = format!("keybind:{}", quote(&arg(1)));
            runs.push(styled_run(None, content, stack));
            return Some(());
        }
        "lang" | "tr" | "translate" if args.len() >= 2 => {
            let mut content = format!("translate:{}", quote(&arg(1)));
            if args.len() > 2 {
                let with: Vec<String> = args[2..].iter().map(|a| to_snbt(a)).collect();
                content += &format!(",with:[{}]", with.join(","));
            }
            runs.push(styled_run(None, content, stack));
            return Some(());
        }
        "click" if args.len() >= 3 => {
            let value = args[2..].join(":");
            let field = match arg(1).as_str() {
                "run_command" | "suggest_command" => "command",
                "open_url" => "url",
                "copy_to_clipboard" => "value",
                "change_page" if value.parse::<i32>().is_ok() => "page",
                _ => return None,
            };
            let value = if field == "page" {
                value
            } else {
                quote(&value)
            };
            style.push((
                "click_event",
                format!("{{action:{},{field}:{value}}}", quote(&arg(1))),
            ));
        }
        "hover" if args.len() >= 3 && arg(1) == "show_text" => {
            let value = to_snbt(&args[2..].join(":"));
            style.push((
                "hover_event",
                format!("{{action:\"show_text\",value:{value}}}"),
            ));
        }
        "insert" | "insertion" if args.len() >= 2 => {
            style.push(("insertion", quote(&args[1..].join(":"))));
        }
        "font" if args.len() == 2 => style.push(("font", quote(&arg(1)))),
        "gradient" => {
            let colors: Option<Vec<u32>> = args[1..].iter().map(|a| rgb(a)).collect();
            let mut colors = colors.filter(|c| c.len() != 1)?;
            if colors.is_empty() {
                colors = vec![0xFFFFFF, 0x000000];
            }
            gradients.push(Gradient::Colors(colors));
            gradient = Some(gradients.len() - 1);
        }
        "rainbow" if args.len() == 1 => {
            gradients.push(Gradient::Rainbow);
            gradient = Some(gradients.len() - 1);
        }
        "color" | "colour" | "c" if args.len() == 2 => {
            style.push(("color", quote(&color_name(&arg(1).to_lowercase())?)));
        }
        _ => {
            let (negate, name) = match tag.strip_prefix('!') {
                Some(name) => (true, name),
                None => (false, tag.as_str()),
            };
            if let Some(decoration) = decoration(name) {
                let on = !negate && arg(1) != "false";
                style.push((decoration, if on { "1b" } else { "0b" }.into()));
            } else if !negate && args.len() == 1 {
                style.push(("color", quote(&color_name(&tag)?)));
            } else {
                return None;
            }
        }
    }
    stack.push(Open {
        name: canonical(&tag),
        style,
        gradient,
    });
    Some(())
}

fn canonical(tag: &str) -> String {
    let tag = tag.trim_start_matches('!');
    if let Some(decoration) = decoration(tag) {
        return decoration.into();
    }
    match tag {
        "colour" | "c" => "color".into(),
        "insert" => "insertion".into(),
        _ if color_name(tag).is_some() => "color".into(),
        _ => tag.into(),
    }
}

fn decoration(tag: &str) -> Option<&'static str> {
    Some(match tag {
        "bold" | "b" => "bold",
        "italic" | "i" | "em" => "italic",
        "underlined" | "u" => "underlined",
        "strikethrough" | "st" => "strikethrough",
        "obfuscated" | "obf" => "obfuscated",
        _ => return None,
    })
}

fn color_name(tag: &str) -> Option<String> {
    let tag = tag.replace("grey", "gray");
    if NAMED_COLORS.contains(&tag.as_str()) {
        return Some(tag);
    }
    rgb(&tag).map(|rgb| format!("#{rgb:06x}"))
}

fn rgb(tag: &str) -> Option<u32> {
    let tag = tag.to_lowercase().replace("grey", "gray");
    if let Some(index) = NAMED_COLORS.iter().position(|name| *name == tag) {
        return Some(NAMED_RGB[index]);
    }
    let hex = tag.strip_prefix('#')?;
    (hex.len() == 6).then(|| u32::from_str_radix(hex, 16).ok())?
}

fn quote(text: &str) -> String {
    let mut out = String::from("\"");
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Splits gradient runs into one run per character with its own color.
fn expand_gradients(runs: Vec<Run>, gradients: &[Gradient]) -> Vec<Run> {
    let mut totals = vec![0usize; gradients.len()];
    for run in &runs {
        if let (Some(g), Some(text)) = (run.gradient, &run.text) {
            totals[g] += text.chars().count();
        }
    }
    let mut seen = vec![0usize; gradients.len()];
    let mut out = Vec::new();
    for run in runs {
        let (Some(g), Some(text)) = (run.gradient, &run.text) else {
            out.push(run);
            continue;
        };
        for c in text.chars() {
            let t = if totals[g] > 1 {
                seen[g] as f64 / (totals[g] - 1) as f64
            } else {
                0.0
            };
            seen[g] += 1;
            let mut style = run.style.clone();
            style.retain(|(k, _)| *k != "color");
            style.push((
                "color",
                quote(&format!("#{:06x}", gradient_color(&gradients[g], t))),
            ));
            out.push(Run {
                content: String::new(),
                text: Some(c.to_string()),
                style,
                gradient: None,
            });
        }
    }
    out
}

fn gradient_color(gradient: &Gradient, t: f64) -> u32 {
    match gradient {
        Gradient::Colors(colors) => {
            let segments = (colors.len() - 1) as f64;
            let at = (t * segments).min(segments - 1e-9).max(0.0);
            let (a, b) = (colors[at as usize], colors[at as usize + 1]);
            let local = at.fract();
            let channel = |shift: u32| {
                let (x, y) = (((a >> shift) & 255) as f64, ((b >> shift) & 255) as f64);
                ((x + (y - x) * local).round() as u32) << shift
            };
            channel(16) | channel(8) | channel(0)
        }
        Gradient::Rainbow => {
            let h = t * 6.0;
            let x = 1.0 - (h % 2.0 - 1.0).abs();
            let (r, g, b) = match h as u32 {
                0 => (1.0, x, 0.0),
                1 => (x, 1.0, 0.0),
                2 => (0.0, 1.0, x),
                3 => (0.0, x, 1.0),
                4 => (x, 0.0, 1.0),
                _ => (1.0, 0.0, x),
            };
            let byte = |v: f64| (v * 255.0).round() as u32;
            (byte(r) << 16) | (byte(g) << 8) | byte(b)
        }
    }
}

fn render_run(run: &Run) -> String {
    let mut fields = Vec::new();
    match &run.text {
        Some(text) => fields.push(format!("text:{}", quote(text))),
        None => fields.push(run.content.clone()),
    }
    for (key, value) in &run.style {
        fields.push(format!("{key}:{value}"));
    }
    format!("{{{}}}", fields.join(","))
}

#[cfg(test)]
mod tests {
    use super::to_snbt;

    #[test]
    fn parses_minimessage() {
        assert_eq!(
            to_snbt("<red>Hi <b>there</b>!"),
            r##"{text:"",extra:[{text:"Hi ",color:"red"},{text:"there",color:"red",bold:1b},{text:"!",color:"red"}]}"##
        );
        assert_eq!(
            to_snbt("<click:run_command:'/say hi'><hover:show_text:'<green>go'>x"),
            r##"{text:"",extra:[{text:"x",click_event:{action:"run_command",command:"/say hi"},hover_event:{action:"show_text",value:{text:"",extra:[{text:"go",color:"green"}]}}}]}"##
        );
        assert_eq!(
            to_snbt("a <nope> \\<red> <#ff0000>b<reset>c<br>"),
            r##"{text:"",extra:[{text:"a <nope> <red> "},{text:"b",color:"#ff0000"},{text:"c\n"}]}"##
        );
        assert_eq!(
            to_snbt("<gradient:red:blue>abc"),
            r##"{text:"",extra:[{text:"a",color:"#ff5555"},{text:"b",color:"#aa55aa"},{text:"c",color:"#5555ff"}]}"##
        );
        assert_eq!(
            to_snbt("<lang:item.minecraft.stone> <key:key.jump>"),
            r##"{text:"",extra:[{translate:"item.minecraft.stone"},{text:" "},{keybind:"key.jump"}]}"##
        );
    }
}
