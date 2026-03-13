use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};

/// Parsed tmux pane layout tree.
/// Tmux layout strings look like:
///   24x80,0,0,0                        (single pane)
///   24x80,0,0[24x40,0,0,0,24x39,0,41,1] (vertical split)
///   24x80,0,0{40x80,0,0,0,39x80,41,0,1} (horizontal split)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PaneLayout {
    pub cols: u16,
    pub rows: u16,
    pub x: u16,
    pub y: u16,
    pub node: LayoutNode,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LayoutNode {
    /// A leaf pane with its tmux pane ID
    Pane { pane_id: u32 },
    /// Vertical split: children stacked top-to-bottom  `[...]`
    VSplit { children: Vec<PaneLayout> },
    /// Horizontal split: children side-by-side  `{...}`
    HSplit { children: Vec<PaneLayout> },
}

/// Parse a tmux layout string into a `PaneLayout` tree.
pub fn parse_layout(s: &str) -> Result<PaneLayout> {
    let s = s.trim();
    // Skip optional checksum prefix: "abc1,24x80,..."
    let s = if s.contains(',') && s.split(',').next().map_or(false, |p| {
        p.chars().all(|c| c.is_ascii_alphanumeric()) && p.len() == 4
    }) {
        s.splitn(2, ',').nth(1).unwrap_or(s)
    } else {
        s
    };

    parse_node(s).map(|(layout, _)| layout)
}

/// Returns (layout, remaining_str)
fn parse_node(s: &str) -> Result<(PaneLayout, &str)> {
    // Parse "COLSxROWS,X,Y" prefix
    let (header, rest) = split_header(s)?;
    let (cols, rows, x, y) = parse_dimensions(header)?;

    let (node, rest) = match rest.chars().next() {
        Some('[') => {
            // Vertical split
            let (children, rest) = parse_children(&rest[1..], ']')?;
            (LayoutNode::VSplit { children }, rest)
        }
        Some('{') => {
            // Horizontal split
            let (children, rest) = parse_children(&rest[1..], '}')?;
            (LayoutNode::HSplit { children }, rest)
        }
        _ => {
            // Leaf pane — next token is the pane ID
            let (pane_id, rest) = parse_pane_id(rest)?;
            (LayoutNode::Pane { pane_id }, rest)
        }
    };

    Ok((PaneLayout { cols, rows, x, y, node }, rest))
}

fn parse_children<'a>(s: &'a str, close: char) -> Result<(Vec<PaneLayout>, &'a str)> {
    let mut children = vec![];
    let mut rem = s;

    loop {
        if rem.starts_with(close) {
            return Ok((children, &rem[1..]));
        }
        if rem.starts_with(',') {
            rem = &rem[1..];
        }
        let (child, rest) = parse_node(rem)?;
        children.push(child);
        rem = rest;
    }
}

fn split_header(s: &str) -> Result<(&str, &str)> {
    // Header ends at '[', '{', ',' (for pane id) or end of string
    let end = s.find(|c| c == '[' || c == '{')
        .unwrap_or(s.len());

    // The header is "COLSxROWS,X,Y" — find where pane_id starts (after 3 commas)
    let mut comma_count = 0;
    let mut header_end = end;
    for (i, c) in s[..end].char_indices() {
        if c == ',' {
            comma_count += 1;
            if comma_count == 3 {
                header_end = i;
                break;
            }
        }
    }

    Ok((&s[..header_end], &s[header_end..]))
}

fn parse_dimensions(s: &str) -> Result<(u16, u16, u16, u16)> {
    let parts: Vec<&str> = s.split(',').collect();
    if parts.len() < 3 {
        return Err(anyhow!("expected at least 3 parts in layout header, got: {}", s));
    }
    let (cols_s, rows_s) = parts[0].split_once('x')
        .ok_or_else(|| anyhow!("expected COLSxROWS, got: {}", parts[0]))?;
    Ok((
        cols_s.parse()?,
        rows_s.parse()?,
        parts[1].parse()?,
        parts[2].parse()?,
    ))
}

fn parse_pane_id(s: &str) -> Result<(u32, &str)> {
    let s = s.strip_prefix(',').unwrap_or(s);
    let end = s.find(|c: char| !c.is_ascii_digit()).unwrap_or(s.len());
    if end == 0 {
        return Ok((0, s)); // pane id is optional at end
    }
    let id: u32 = s[..end].parse()?;
    Ok((id, &s[end..]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_pane() {
        let layout = parse_layout("24x80,0,0,0").unwrap();
        assert_eq!(layout.cols, 80);
        assert_eq!(layout.rows, 24);
        assert!(matches!(layout.node, LayoutNode::Pane { pane_id: 0 }));
    }

    #[test]
    fn vertical_split() {
        // Two panes stacked vertically
        let layout = parse_layout("24x80,0,0[12x80,0,0,0,11x80,0,13,1]").unwrap();
        assert!(matches!(layout.node, LayoutNode::VSplit { ref children } if children.len() == 2));
    }

    #[test]
    fn horizontal_split() {
        let layout = parse_layout("24x80,0,0{40x24,0,0,0,39x24,41,0,1}").unwrap();
        assert!(matches!(layout.node, LayoutNode::HSplit { ref children } if children.len() == 2));
    }

    #[test]
    fn nested_split() {
        // Left/right, right side split top/bottom
        let s = "24x80,0,0{40x24,0,0,0,39x24,41,0[20x39,41,0,1,18x39,41,21,2]}";
        let layout = parse_layout(s).unwrap();
        if let LayoutNode::HSplit { children } = &layout.node {
            assert_eq!(children.len(), 2);
            assert!(matches!(&children[1].node, LayoutNode::VSplit { children } if children.len() == 2));
        } else {
            panic!("expected HSplit");
        }
    }
}
