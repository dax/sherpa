use pulldown_cmark::{html, Options, Parser};

/// Convert a markdown string to HTML.
pub fn to_html(markdown: &str) -> String {
    let mut options = Options::empty();
    options.insert(Options::ENABLE_STRIKETHROUGH);
    options.insert(Options::ENABLE_TABLES);
    let parser = Parser::new_ext(markdown, options);
    let mut html_output = String::new();
    html::push_html(&mut html_output, parser);
    html_output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_basic_markdown() {
        let result = to_html("**bold** and *italic*");
        assert!(result.contains("<strong>bold</strong>"));
        assert!(result.contains("<em>italic</em>"));
    }

    #[test]
    fn test_code_block() {
        let result = to_html("```rust\nfn main() {}\n```");
        assert!(result.contains("<code"));
        assert!(result.contains("fn main()"));
    }

    #[test]
    fn test_plain_text_wraps_in_p() {
        let result = to_html("Hello world");
        assert!(result.contains("<p>Hello world</p>"));
    }

    #[test]
    fn test_bullet_list() {
        let result = to_html("- item 1\n- item 2");
        assert!(result.contains("<li>"));
        assert!(result.contains("item 1"));
    }
}
