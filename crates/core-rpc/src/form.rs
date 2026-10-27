//! A PDF form — an AcroForm, fields laid over the page to type into — read
//! and filled.
//!
//! [`fields`] lists what a form asks: each field's name, what kind it is, the
//! label the form's author gave it, its page, and what is in it. The names
//! are the form's own (`Vorname`, `ja_3`), so together with the page's text a
//! model can tell which question each belongs to.
//!
//! [`fill`] writes values into a copy. A text field gets its text, a checkbox
//! its cross, and each filled widget a fresh appearance stream, so every
//! reader shows the values — including the ones that ignore NeedAppearances,
//! which is set too for the ones that honour it. The original is never
//! touched: filling makes new bytes.

use std::collections::BTreeMap;

use lopdf::{Dictionary, Document, Object, ObjectId, Stream, StringFormat};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::pdf;

/// One field of a form, as the assistant is shown it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FormField {
    /// The fully qualified name, which is what `fill` takes.
    pub name: String,
    /// `text`, `checkbox`, `radio` or `choice`.
    pub kind: String,
    /// The label the form's author wrote for the field (its tooltip), which
    /// often says more than the name.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// What is in it now.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    /// The page the field is on, 1-based.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub page: Option<usize>,
    /// A checkbox's state when ticked; a radio's or a choice's options.
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub options: Vec<String>,
    #[serde(skip_serializing_if = "std::ops::Not::not", default)]
    pub multiline: bool,
}

/// A form filled: the new bytes and how many fields were written.
#[derive(Debug)]
pub struct Filled {
    pub pdf: Vec<u8>,
    pub filled: usize,
}

/// The field flags that matter here.
const FF_MULTILINE: i64 = 1 << 12;
const FF_PUSHBUTTON: i64 = 1 << 16;
const FF_RADIO: i64 = 1 << 15;

/// One widget of a field: where it is drawn, and — for a button — the name
/// of its ticked state.
#[derive(Debug, Clone)]
struct Widget {
    id: ObjectId,
    rect: Option<[f32; 4]>,
    on: Option<String>,
}

/// A terminal field: the dictionary holding its name and value, with its
/// widgets, as one pass over the field tree collects them.
#[derive(Debug, Clone)]
struct Node {
    id: ObjectId,
    name: String,
    label: Option<String>,
    /// `Tx`, `Btn` or `Ch`.
    ft: Vec<u8>,
    flags: i64,
    da: Option<String>,
    value: Option<String>,
    options: Vec<String>,
    widgets: Vec<Widget>,
}

impl Node {
    fn kind(&self) -> &'static str {
        match self.ft.as_slice() {
            b"Tx" => "text",
            b"Ch" => "choice",
            b"Btn" if self.flags & FF_RADIO != 0 || self.states().len() > 1 => "radio",
            _ => "checkbox",
        }
    }

    fn multiline(&self) -> bool {
        self.ft == b"Tx" && self.flags & FF_MULTILINE != 0
    }

    /// The distinct ticked states of its widgets: one for a checkbox, one per
    /// option for a radio group.
    fn states(&self) -> Vec<String> {
        let mut states: Vec<String> = Vec::new();
        for widget in &self.widgets {
            if let Some(on) = &widget.on {
                if !states.contains(on) {
                    states.push(on.clone());
                }
            }
        }
        states
    }
}

/// A PDF text string decoded: UTF-16 behind its byte-order mark, otherwise
/// read as Latin-1, which is what PDFDocEncoding is where German needs it.
fn decode(bytes: &[u8]) -> String {
    if bytes.starts_with(&[0xFE, 0xFF]) {
        let units: Vec<u16> = bytes[2..]
            .chunks_exact(2)
            .map(|pair| u16::from_be_bytes([pair[0], pair[1]]))
            .collect();
        String::from_utf16_lossy(&units)
    } else {
        bytes.iter().map(|&b| b as char).collect()
    }
}

/// A PDF text string for a value: UTF-16 with the byte-order mark, as hex,
/// so any character survives and nothing needs escaping.
fn text_string(text: &str) -> Object {
    let mut bytes = vec![0xFE, 0xFF];
    for unit in text.encode_utf16() {
        bytes.extend_from_slice(&unit.to_be_bytes());
    }
    Object::String(bytes, StringFormat::Hexadecimal)
}

/// `key` of `dict`, with a reference followed to what it names.
fn resolved<'a>(doc: &'a Document, dict: &'a Dictionary, key: &[u8]) -> Option<&'a Object> {
    let value = dict.get(key).ok()?;
    doc.dereference(value).ok().map(|(_, object)| object)
}

fn string_of(object: &Object) -> Option<String> {
    match object {
        Object::String(bytes, _) => Some(decode(bytes)),
        _ => None,
    }
}

/// What is in a field: the text of a string, the state of a button — `Off`
/// read as empty.
fn value_of(object: &Object) -> Option<String> {
    match object {
        Object::String(bytes, _) => Some(decode(bytes)),
        Object::Name(bytes) if bytes != b"Off" => Some(decode(bytes)),
        _ => None,
    }
}

/// The widget's ticked state: the key of its normal appearance that is not
/// `Off`.
fn on_state(doc: &Document, widget: &Dictionary) -> Option<String> {
    let normal = resolved(doc, widget, b"AP")
        .and_then(|ap| ap.as_dict().ok())
        .and_then(|ap| resolved_from(doc, ap.get(b"N").ok()?))?
        .as_dict()
        .ok()?
        .clone();
    normal
        .iter()
        .map(|(key, _)| decode(key))
        .find(|state| state != "Off")
}

fn resolved_from<'a>(doc: &'a Document, object: &'a Object) -> Option<&'a Object> {
    doc.dereference(object).ok().map(|(_, object)| object)
}

/// The options of a choice field: each entry a string, or a pair whose first
/// is the export value.
fn choice_options(doc: &Document, field: &Dictionary) -> Vec<String> {
    let Some(Object::Array(entries)) = resolved(doc, field, b"Opt") else {
        return Vec::new();
    };
    entries
        .iter()
        .filter_map(|entry| match resolved_from(doc, entry)? {
            Object::String(bytes, _) => Some(decode(bytes)),
            Object::Array(pair) => string_of(resolved_from(doc, pair.first()?)?),
            _ => None,
        })
        .collect()
}

/// Every terminal field of the form, in the order of the field tree. An
/// empty list is a flat PDF: nothing to type into.
fn collect(doc: &Document) -> Result<Vec<Node>, String> {
    let catalog = doc
        .catalog()
        .map_err(|err| format!("this PDF has no catalog: {err}"))?;
    let Some(acro) = resolved(doc, catalog, b"AcroForm").and_then(|a| a.as_dict().ok()) else {
        return Ok(Vec::new());
    };
    let form_da = acro.get(b"DA").ok().and_then(string_of);
    let Some(Object::Array(roots)) = resolved(doc, acro, b"Fields") else {
        return Ok(Vec::new());
    };

    struct Inherited {
        prefix: String,
        ft: Option<Vec<u8>>,
        flags: Option<i64>,
        da: Option<String>,
        value: Option<String>,
    }
    let mut out = Vec::new();
    let mut stack: Vec<(ObjectId, Inherited)> = roots
        .iter()
        .rev()
        .filter_map(|entry| entry.as_reference().ok())
        .map(|id| {
            (
                id,
                Inherited {
                    prefix: String::new(),
                    ft: None,
                    flags: None,
                    da: form_da.clone(),
                    value: None,
                },
            )
        })
        .collect();
    let mut seen = std::collections::HashSet::new();
    while let Some((id, inherited)) = stack.pop() {
        if !seen.insert(id) || out.len() + stack.len() > 20_000 {
            continue;
        }
        let Ok(dict) = doc.get_dictionary(id) else {
            continue;
        };
        let own_name = dict.get(b"T").ok().and_then(string_of);
        let name = match (&inherited.prefix, &own_name) {
            (prefix, Some(t)) if prefix.is_empty() => t.clone(),
            (prefix, Some(t)) => format!("{prefix}.{t}"),
            (prefix, None) => prefix.clone(),
        };
        let ft = dict
            .get(b"FT")
            .ok()
            .and_then(|o| o.as_name().ok().map(<[u8]>::to_vec))
            .or(inherited.ft);
        let flags = dict
            .get(b"Ff")
            .ok()
            .and_then(|o| o.as_i64().ok())
            .or(inherited.flags);
        let da = dict.get(b"DA").ok().and_then(string_of).or(inherited.da);
        let value = resolved(doc, dict, b"V")
            .and_then(value_of)
            .or(inherited.value);

        // Kids that are fields of their own (they carry a name) are walked;
        // the rest are this field's widgets.
        let mut widgets = Vec::new();
        let mut is_parent = false;
        if let Some(Object::Array(kids)) = resolved(doc, dict, b"Kids") {
            for kid in kids {
                let Ok(kid_id) = kid.as_reference() else {
                    continue;
                };
                let named = doc.get_dictionary(kid_id).ok().is_some_and(|k| k.has(b"T"));
                if named {
                    is_parent = true;
                    stack.push((
                        kid_id,
                        Inherited {
                            prefix: name.clone(),
                            ft: ft.clone(),
                            flags,
                            da: da.clone(),
                            value: value.clone(),
                        },
                    ));
                } else {
                    widgets.push(kid_id);
                }
            }
        } else if dict
            .get(b"Subtype")
            .ok()
            .and_then(|s| s.as_name().ok())
            .is_some_and(|s| s == b"Widget")
        {
            widgets.push(id);
        }
        if is_parent && widgets.is_empty() {
            continue;
        }
        let Some(ft) = ft else {
            continue;
        };
        let flags = flags.unwrap_or(0);
        // A push button does nothing on paper: it is not a field to fill.
        if ft == b"Btn" && flags & FF_PUSHBUTTON != 0 {
            continue;
        }
        if name.is_empty() {
            continue;
        }
        let widgets = widgets
            .into_iter()
            .filter_map(|wid| {
                let widget = doc.get_dictionary(wid).ok()?;
                Some(Widget {
                    id: wid,
                    rect: pdf::rectangle(resolved(doc, widget, b"Rect")),
                    on: on_state(doc, widget),
                })
            })
            .collect();
        let options = if ft == b"Ch" {
            choice_options(doc, dict)
        } else {
            Vec::new()
        };
        out.push(Node {
            id,
            name,
            label: dict.get(b"TU").ok().and_then(string_of),
            ft,
            flags,
            da,
            value,
            options,
            widgets,
        });
    }
    Ok(out)
}

/// Where a widget is drawn: its page, 1-based, and its place in the reading
/// order of that page.
type Place = (usize, usize);

/// Which page each widget is drawn on, 1-based, and where in the page's
/// reading order, so the fields can be listed as the form reads.
fn placement(doc: &Document) -> BTreeMap<ObjectId, Place> {
    let mut places = BTreeMap::new();
    for (number, page_id) in doc.get_pages() {
        let Ok(page) = doc.get_dictionary(page_id) else {
            continue;
        };
        let Some(Object::Array(annots)) = resolved(doc, page, b"Annots") else {
            continue;
        };
        for (at, annot) in annots.iter().enumerate() {
            if let Ok(id) = annot.as_reference() {
                places.entry(id).or_insert((number as usize, at));
            }
        }
    }
    places
}

/// The fields of a form, in the order its pages show them. Empty is a flat
/// PDF with nothing to type into.
pub fn fields(pdf_bytes: &[u8]) -> Result<Vec<FormField>, String> {
    let doc = pdf::load(pdf_bytes)?;
    let nodes = collect(&doc)?;
    let places = placement(&doc);
    let mut listed: Vec<(Option<Place>, usize, FormField)> = nodes
        .iter()
        .enumerate()
        .map(|(at, node)| {
            let place = node
                .widgets
                .iter()
                .filter_map(|w| places.get(&w.id))
                .min()
                .copied();
            let kind = node.kind();
            let options = match kind {
                "checkbox" | "radio" => node.states(),
                "choice" => node.options.clone(),
                _ => Vec::new(),
            };
            let field = FormField {
                name: node.name.clone(),
                kind: kind.into(),
                label: node.label.clone().filter(|l| *l != node.name),
                value: node.value.clone(),
                page: place.map(|(page, _)| page),
                options,
                multiline: node.multiline(),
            };
            (place, at, field)
        })
        .collect();
    listed.sort_by_key(|(place, at, _)| (place.unwrap_or((usize::MAX, *at)), *at));
    Ok(listed.into_iter().map(|(_, _, field)| field).collect())
}

/// The ticked state a value asks of a button, or `None` for off. `options`
/// are the button's states; a plain yes only works where there is one.
fn button_state(value: &Value, options: &[String]) -> Result<Option<String>, String> {
    let yes = || match options {
        [only] => Ok(Some(only.clone())),
        [] => Ok(Some("Yes".to_string())),
        _ => Err(format!(
            "this is a group: say which of its options to tick: {}",
            options.join(", ")
        )),
    };
    match value {
        Value::Bool(true) => yes(),
        Value::Bool(false) | Value::Null => Ok(None),
        Value::Number(n) if n.as_f64() == Some(1.0) => yes(),
        Value::Number(n) if n.as_f64() == Some(0.0) => Ok(None),
        Value::String(text) => {
            let text = text.trim();
            if let Some(option) = options
                .iter()
                .find(|option| option.eq_ignore_ascii_case(text))
            {
                return Ok(Some(option.clone()));
            }
            match text.to_lowercase().as_str() {
                "x" | "ja" | "yes" | "true" | "on" | "1" | "checked" => yes(),
                "" | "nein" | "no" | "false" | "off" | "0" | "unchecked" => Ok(None),
                other => Err(format!(
                    "{other} is not a state of this field; its options are: {}",
                    if options.is_empty() {
                        "true or false".to_string()
                    } else {
                        options.join(", ")
                    }
                )),
            }
        }
        other => Err(format!("{other} is not a state a checkbox can take")),
    }
}

/// The text a value asks a text field to hold.
fn text_value(value: &Value) -> Result<String, String> {
    match value {
        Value::String(text) => Ok(text.clone()),
        Value::Number(n) => Ok(n.to_string()),
        Value::Null => Ok(String::new()),
        Value::Bool(_) => Err("this is a text field: give it text, not yes or no".into()),
        other => Err(format!("{other} is not text")),
    }
}

/// The font a field draws its text with: the name its default appearance
/// gives, and its size — 0 meaning pick one for the field's height.
fn parse_da(da: &str) -> (Option<String>, f32) {
    let tokens: Vec<&str> = da.split_whitespace().collect();
    let mut font = None;
    let mut size = 0.0;
    for (at, token) in tokens.iter().enumerate() {
        if *token == "Tf" && at >= 2 {
            font = tokens[at - 2].strip_prefix('/').map(str::to_string);
            size = tokens[at - 1].parse().unwrap_or(0.0);
        }
    }
    (font, size)
}

/// The font `name` names in the form's resources, or a Helvetica added to
/// them under `Helv`, so every appearance stream has a font to point at.
fn form_font(doc: &mut Document, name: Option<&str>) -> Result<(String, ObjectId), String> {
    let named = |doc: &Document, name: &str| -> Option<ObjectId> {
        let catalog = doc.catalog().ok()?;
        let acro = resolved(doc, catalog, b"AcroForm")?.as_dict().ok()?;
        let fonts = resolved(doc, acro, b"DR")?.as_dict().ok()?;
        let fonts = resolved(doc, fonts, b"Font")?.as_dict().ok()?;
        fonts.get(name.as_bytes()).ok()?.as_reference().ok()
    };
    if let Some(name) = name {
        if let Some(id) = named(doc, name) {
            return Ok((name.to_string(), id));
        }
    }
    if let Some(id) = named(doc, "Helv") {
        return Ok(("Helv".to_string(), id));
    }
    // No usable font among the form's own: one of ours, left out of the
    // form's resources — each appearance stream carries its own reference.
    let id = doc.add_object(pdf::font("Helvetica"));
    Ok(("Helv".to_string(), id))
}

/// A fresh appearance for a text widget: the value drawn in the field's box,
/// so every reader shows it.
fn text_appearance(
    doc: &mut Document,
    widget: &Widget,
    text: &str,
    da: Option<&str>,
    multiline: bool,
) -> Result<(), String> {
    let [x0, y0, x1, y1] = widget.rect.unwrap_or([0.0, 0.0, 100.0, 14.0]);
    let (w, h) = (x1 - x0, y1 - y0);
    let (font_name, asked) = parse_da(da.unwrap_or_default());
    let (font_name, font_id) = form_font(doc, font_name.as_deref())?;
    let mut size = if asked > 0.0 {
        asked
    } else if multiline {
        10.0_f32.min((h * 0.72).max(6.0))
    } else {
        (h * 0.72).clamp(6.0, 16.0)
    };
    let room = (w - 4.0).max(4.0);
    let mut body = String::new();
    if multiline {
        let leading = size * 1.2;
        let mut y = h - size - 2.0;
        for line in text.split('\n').flat_map(|paragraph| {
            if paragraph.trim().is_empty() {
                vec![String::new()]
            } else {
                pdf::wrap(paragraph, size, false, room)
            }
        }) {
            if y < 1.0 {
                break;
            }
            if !line.is_empty() {
                body.push_str(&format!(
                    "BT /{font_name} {size:.2} Tf 0 g 2 {y:.2} Td {} Tj ET\n",
                    pdf::literal(&line)
                ));
            }
            y -= leading;
        }
    } else {
        let line = text.replace(['\n', '\r'], " ");
        let wide = pdf::width(&line, size, false);
        if wide > room {
            size = (size * room / wide).max(5.0);
        }
        let y = ((h - size) / 2.0 + size * 0.18).max(1.5);
        body.push_str(&format!(
            "BT /{font_name} {size:.2} Tf 0 g 2 {y:.2} Td {} Tj ET\n",
            pdf::literal(&line)
        ));
    }
    let content = format!("/Tx BMC\nq\n{body}Q\nEMC\n");

    let mut fonts = Dictionary::new();
    fonts.set(font_name.as_bytes(), font_id);
    let mut resources = Dictionary::new();
    resources.set("Font", fonts);
    let mut dict = Dictionary::new();
    dict.set("Type", Object::Name(b"XObject".to_vec()));
    dict.set("Subtype", Object::Name(b"Form".to_vec()));
    dict.set(
        "BBox",
        Object::Array(vec![0.into(), 0.into(), Object::Real(w), Object::Real(h)]),
    );
    dict.set("Resources", resources);
    let appearance = doc.add_object(Stream::new(dict, content.into_bytes()));

    let widget_dict = doc
        .get_object_mut(widget.id)
        .and_then(Object::as_dict_mut)
        .map_err(|err| format!("a field of this form cannot be changed: {err}"))?;
    let mut ap = Dictionary::new();
    ap.set("N", appearance);
    widget_dict.set("AP", ap);
    Ok(())
}

/// Sets NeedAppearances on the form, for the readers that honour it.
fn need_appearances(doc: &mut Document) -> Result<(), String> {
    let root_id = doc
        .trailer
        .get(b"Root")
        .and_then(Object::as_reference)
        .map_err(|err| format!("this PDF has no catalog: {err}"))?;
    let catalog = doc.get_dictionary(root_id).map_err(|e| e.to_string())?;
    match catalog.get(b"AcroForm").map(Object::as_reference) {
        Ok(Ok(form_id)) => {
            doc.get_object_mut(form_id)
                .and_then(Object::as_dict_mut)
                .map_err(|e| e.to_string())?
                .set("NeedAppearances", true);
        }
        Ok(Err(_)) => {
            let mut form = catalog
                .get(b"AcroForm")
                .ok()
                .and_then(|f| f.as_dict().ok())
                .cloned()
                .unwrap_or_default();
            form.set("NeedAppearances", true);
            doc.get_object_mut(root_id)
                .and_then(Object::as_dict_mut)
                .map_err(|e| e.to_string())?
                .set("AcroForm", form);
        }
        Err(_) => {}
    }
    Ok(())
}

/// Writes `values` — field name to text, or to yes/no for a checkbox — into
/// a copy of the form. A name the form does not have refuses the whole call,
/// so what comes back is never half of what was asked.
pub fn fill(pdf_bytes: &[u8], values: &serde_json::Map<String, Value>) -> Result<Filled, String> {
    if values.is_empty() {
        return Err("values is empty: nothing to fill in".into());
    }
    let mut doc = pdf::load(pdf_bytes)?;
    let nodes = collect(&doc)?;
    if nodes.is_empty() {
        return Err(
            "this PDF has no form fields to fill: it is a flat page. Read it with \
             read_attachment; what it asks for can only be written into the reply"
                .into(),
        );
    }
    let mut by_name: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
    for (at, node) in nodes.iter().enumerate() {
        by_name.entry(node.name.as_str()).or_default().push(at);
    }
    let unknown: Vec<&str> = values
        .keys()
        .map(String::as_str)
        .filter(|name| !by_name.contains_key(name))
        .collect();
    if !unknown.is_empty() {
        return Err(format!(
            "this form has no field named {}; read_form lists the fields by name",
            unknown.join(", ")
        ));
    }

    let mut filled = 0;
    for (name, value) in values {
        for &at in &by_name[name.as_str()] {
            let node = nodes[at].clone();
            let against = |err: String| format!("{name}: {err}");
            if node.ft == b"Btn" {
                let state = button_state(value, &node.states()).map_err(against)?;
                let chosen = state.as_deref().unwrap_or("Off");
                doc.get_object_mut(node.id)
                    .and_then(Object::as_dict_mut)
                    .map_err(|err| against(err.to_string()))?
                    .set("V", Object::Name(chosen.as_bytes().to_vec()));
                for widget in &node.widgets {
                    // In a group, only the widget whose ticked state was
                    // chosen shows it; every other one shows off.
                    let shown = if widget.on.as_deref() == Some(chosen) || node.widgets.len() == 1 {
                        chosen
                    } else {
                        "Off"
                    };
                    doc.get_object_mut(widget.id)
                        .and_then(Object::as_dict_mut)
                        .map_err(|err| against(err.to_string()))?
                        .set("AS", Object::Name(shown.as_bytes().to_vec()));
                }
            } else {
                let text = text_value(value).map_err(against)?;
                doc.get_object_mut(node.id)
                    .and_then(Object::as_dict_mut)
                    .map_err(|err| against(err.to_string()))?
                    .set("V", text_string(&text));
                for widget in &node.widgets {
                    text_appearance(
                        &mut doc,
                        widget,
                        &text,
                        node.da.as_deref(),
                        node.multiline(),
                    )
                    .map_err(against)?;
                }
            }
        }
        filled += 1;
    }
    need_appearances(&mut doc)?;
    let mut out = Vec::new();
    doc.save_to(&mut out)
        .map_err(|err| format!("the filled PDF could not be written: {err}"))?;
    Ok(Filled { pdf: out, filled })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_default_appearance_names_the_font_and_the_size() {
        assert_eq!(parse_da("/Helv 0 Tf 0 g"), (Some("Helv".into()), 0.0));
        assert_eq!(parse_da("/HeBo 12 Tf 0 g"), (Some("HeBo".into()), 12.0));
        assert_eq!(parse_da("0 g /Helv  8.5  Tf"), (Some("Helv".into()), 8.5));
        assert_eq!(parse_da(""), (None, 0.0));
    }

    #[test]
    fn a_checkbox_takes_yes_no_or_its_own_state() {
        let on = vec!["On".to_string()];
        assert_eq!(button_state(&json!(true), &on).unwrap(), Some("On".into()));
        assert_eq!(button_state(&json!(false), &on).unwrap(), None);
        assert_eq!(button_state(&json!("ja"), &on).unwrap(), Some("On".into()));
        assert_eq!(button_state(&json!("On"), &on).unwrap(), Some("On".into()));
        assert_eq!(button_state(&json!("nein"), &on).unwrap(), None);
        // A group needs to be told which, and names its options.
        let group = vec!["Vollzeit".to_string(), "Teilzeit".to_string()];
        assert!(button_state(&json!(true), &group)
            .unwrap_err()
            .contains("Vollzeit"));
        assert_eq!(
            button_state(&json!("teilzeit"), &group).unwrap(),
            Some("Teilzeit".into())
        );
        // No appearance states stored: the spec's default.
        assert_eq!(button_state(&json!(true), &[]).unwrap(), Some("Yes".into()));
    }

    #[test]
    fn strings_survive_the_trip_through_utf16() {
        let Object::String(bytes, _) = text_string("Müller-Lüdenscheidt, 42 €") else {
            panic!("not a string");
        };
        assert_eq!(decode(&bytes), "Müller-Lüdenscheidt, 42 €");
    }
}
