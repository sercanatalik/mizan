//! A small XML element tree for native adapters.

use quick_xml::events::Event;
use quick_xml::Reader;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Node {
    pub name: String,
    pub attrs: Vec<(String, String)>,
    pub text: String,
    pub children: Vec<Node>,
}

impl Node {
    pub fn child(&self, name: &str) -> Option<&Node> {
        self.children.iter().find(|c| c.name == name)
    }

    pub fn children_named<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a Node> + 'a {
        self.children.iter().filter(move |c| c.name == name)
    }

    /// Text of a descendant reached by a path of child names.
    pub fn text_at(&self, path: &[&str]) -> Option<&str> {
        let mut n = self;
        for p in path {
            n = n.child(p)?;
        }
        let t = n.text.trim();
        (!t.is_empty()).then_some(t)
    }

    /// First descendant (depth-first) with this name.
    pub fn find(&self, name: &str) -> Option<&Node> {
        if self.name == name {
            return Some(self);
        }
        self.children.iter().find_map(|c| c.find(name))
    }

    /// All descendants (depth-first) with this name.
    pub fn find_all<'a>(&'a self, name: &str, out: &mut Vec<&'a Node>) {
        if self.name == name {
            out.push(self);
        }
        for c in &self.children {
            c.find_all(name, out);
        }
    }

    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attrs
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
            .filter(|v| !v.trim().is_empty())
    }
}

/// Parse an XML document into a tree; the returned node is a synthetic root.
pub fn parse(xml: &str) -> Result<Node, String> {
    // Text is kept untrimmed so spaces around entity references survive; readers trim.
    let mut reader = Reader::from_str(xml);
    let mut stack = vec![Node {
        name: "#root".into(),
        ..Default::default()
    }];
    let attrs = |e: &quick_xml::events::BytesStart| -> Vec<(String, String)> {
        e.attributes()
            .flatten()
            .map(|a| {
                let v = a
                    .unescape_value()
                    .map(|v| v.to_string())
                    .unwrap_or_else(|_| String::from_utf8_lossy(&a.value).to_string());
                (String::from_utf8_lossy(a.key.as_ref()).to_string(), v)
            })
            .collect()
    };
    loop {
        match reader
            .read_event()
            .map_err(|e| format!("XML error at byte {}: {e}", reader.buffer_position()))?
        {
            Event::Start(e) => stack.push(Node {
                name: String::from_utf8_lossy(e.name().as_ref()).to_string(),
                attrs: attrs(&e),
                ..Default::default()
            }),
            Event::Empty(e) => {
                let n = Node {
                    name: String::from_utf8_lossy(e.name().as_ref()).to_string(),
                    attrs: attrs(&e),
                    ..Default::default()
                };
                stack.last_mut().expect("root").children.push(n);
            }
            Event::Text(t) => {
                let s = t.decode().map_err(|e| e.to_string())?;
                stack.last_mut().expect("root").text.push_str(&s);
            }
            Event::GeneralRef(r) => {
                let name = String::from_utf8_lossy(r.as_ref()).to_string();
                let ch = match name.as_str() {
                    "amp" => "&",
                    "lt" => "<",
                    "gt" => ">",
                    "quot" => "\"",
                    "apos" => "'",
                    _ => "",
                };
                stack.last_mut().expect("root").text.push_str(ch);
            }
            Event::CData(t) => stack
                .last_mut()
                .expect("root")
                .text
                .push_str(&String::from_utf8_lossy(&t)),
            Event::End(_) => {
                let n = stack.pop().expect("balanced");
                if stack.is_empty() {
                    return Err("unbalanced XML".into());
                }
                stack.last_mut().expect("root").children.push(n);
            }
            Event::Eof => break,
            _ => {}
        }
    }
    if stack.len() != 1 {
        return Err(format!(
            "unclosed element <{}>",
            stack.last().map(|n| n.name.as_str()).unwrap_or("?")
        ));
    }
    Ok(stack.pop().expect("root"))
}
