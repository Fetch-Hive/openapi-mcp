use mcp_gateway_ir::Style;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StyleSupport {
    Ok(Style, bool),
    Unsupported { blocking_if_required: bool },
}

pub fn parse_style(location: &str, style: Option<&str>, explode: Option<bool>) -> StyleSupport {
    let loc = location.to_ascii_lowercase();
    let default_style = match loc.as_str() {
        "query" | "cookie" => "form",
        _ => "simple",
    };
    let style_name = style.unwrap_or(default_style);
    let default_explode = style_name == "form";
    let explode = explode.unwrap_or(default_explode);

    match (style_name, loc.as_str()) {
        ("simple", "path" | "header") => StyleSupport::Ok(Style::Simple, explode),
        ("form", "query" | "cookie") => StyleSupport::Ok(Style::Form, explode),
        ("spaceDelimited", "query") => StyleSupport::Ok(Style::SpaceDelimited, explode),
        ("pipeDelimited", "query") => StyleSupport::Ok(Style::PipeDelimited, explode),
        ("label" | "matrix", "path") => {
            let st = if style_name == "label" {
                Style::Label
            } else {
                Style::Matrix
            };
            StyleSupport::Ok(st, explode)
        }
        ("deepObject", _) => StyleSupport::Unsupported {
            blocking_if_required: true,
        },
        _ => StyleSupport::Unsupported {
            blocking_if_required: true,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deep_object_unsupported() {
        assert!(matches!(
            parse_style("query", Some("deepObject"), Some(true)),
            StyleSupport::Unsupported { .. }
        ));
    }
}
