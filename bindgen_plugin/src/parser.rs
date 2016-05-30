use syntax::ast;
use syntax::ext::base;
use syntax::fold::Folder;
use syntax::parse::token;

use bindgen::{BindgenOptions, LinkType};

// Parses macro invocations in the form [ident=|:]value where value is an ident or literal
// e.g. bindgen!(module_name, "header.h", builtins=false, clang_args:"-I /usr/local/include")
pub fn parse_macro_opts(cx: &mut base::ExtCtxt,
                    tts: &[ast::TokenTree],
                    options: &mut BindgenOptions)
                    -> bool {

    let mut visit = BindgenArgsVisitor {options: options, seen_named: false};
    let mut parser = cx.new_parser_from_tts(tts);
    let mut args_good = true;

    loop {
        let mut name: Option<String> = None;
        let mut span = parser.span;

        // Check for [ident=]value and if found save ident to name
        if parser.look_ahead(1, |t| t == &token::Eq) {
            match parser.bump_and_get() {
                token::Token::Ident(ident) => {
                    let ident = parser.id_to_interned_str(ident);
                    name = Some(ident.to_string());
                    if let Err(_) = parser.expect(&token::Eq) {
                        return false;
                    }
                }
                _ => {
                    cx.span_err(span, "invalid argument format");
                    return false;
                }
            }
        }

        match parser.token {
            // Match [ident]
            token::Token::Ident(val) => {
                let val = parser.id_to_interned_str(val);
                span.hi = parser.span.hi;
                parser.bump();

                // Bools are simply encoded as idents
                let ret = match &*val {
                    "true" => visit.visit_bool(as_str(&name), true),
                    "false" => visit.visit_bool(as_str(&name), false),
                    val => visit.visit_ident(as_str(&name), val),
                };
                if !ret {
                    cx.span_err(span, "invalid argument");
                    args_good = false;
                }
            }
            // Match [literal] and parse as an expression so we can expand macros
            _ => {
                let expr = cx.expander().fold_expr(parser.parse_expr().unwrap());
                span.hi = expr.span.hi;
                match expr.node {
                    ast::ExprKind::Lit(ref lit) => {
                        let ret = match lit.node {
                            ast::LitKind::Str(ref s, _) => visit.visit_str(as_str(&name), &*s),
                            ast::LitKind::Bool(b) => visit.visit_bool(as_str(&name), b),
                            ast::LitKind::Int(i, ast::LitIntType::Unsigned(_)) |
                            ast::LitKind::Int(i, ast::LitIntType::Unsuffixed) => {
                                visit.visit_uint(as_str(&name), i)
                            }
                            ast::LitKind::Int(i, ast::LitIntType::Signed(_)) => {
                                visit.visit_int(as_str(&name), i as i64)
                            }
                            _ => {
                                cx.span_err(span, "invalid argument format");
                                return false;
                            }
                        };
                        if !ret {
                            cx.span_err(span, "invalid argument");
                            args_good = false;
                        }
                    }
                    _ => {
                        cx.span_err(span, "invalid argument format");
                        return false;
                    }
                }
            }
        }

        if parser.check(&token::Eof) {
            return args_good;
        }

        if !parser.eat(&token::Comma) {
            cx.span_err(parser.span, "invalid argument format");
            return false;
        }
    }
}

// I'm sure there's a nicer way of doing it
fn as_str<'a>(owned: &'a Option<String>) -> Option<&'a str> {
    match owned {
        &Some(ref s) => Some(&s[..]),
        &None => None,
    }
}

#[derive(PartialEq, Eq)]
enum QuoteState {
    InNone,
    InSingleQuotes,
    InDoubleQuotes,
}

fn parse_process_args(s: &str) -> Vec<String> {
    let s = s.trim();
    let mut parts = Vec::new();
    let mut quote_state = QuoteState::InNone;
    let mut positions = vec![0];
    let mut last = ' ';
    for (i, c) in s.chars().chain(" ".chars()).enumerate() {
        match (last, c) {
            // Match \" set has_escaped and skip
            ('\\', '\"') => (),
            // Match \'
            ('\\', '\'') => (),
            // Match \<space>
            // Check we don't escape the final added space
            ('\\', ' ') if i < s.len() => (),
            // Match \\
            ('\\', '\\') => (),
            // Match <any>"
            (_, '\"') if quote_state == QuoteState::InNone => {
                quote_state = QuoteState::InDoubleQuotes;
                positions.push(i);
                positions.push(i + 1);
            }
            (_, '\"') if quote_state == QuoteState::InDoubleQuotes => {
                quote_state = QuoteState::InNone;
                positions.push(i);
                positions.push(i + 1);
            }
            // Match <any>'
            (_, '\'') if quote_state == QuoteState::InNone => {
                quote_state = QuoteState::InSingleQuotes;
                positions.push(i);
                positions.push(i + 1);
            }
            (_, '\'') if quote_state == QuoteState::InSingleQuotes => {
                quote_state = QuoteState::InNone;
                positions.push(i);
                positions.push(i + 1);
            }
            // Match <any><space>
            // If we are at the end of the string close any open quotes
            (_, ' ') if quote_state == QuoteState::InNone || i >= s.len() => {
                {
                    positions.push(i);

                    let starts = positions.iter().enumerate().filter(|&(i, _)| i % 2 == 0);
                    let ends = positions.iter().enumerate().filter(|&(i, _)| i % 2 == 1);

                    let part: Vec<String> = starts.zip(ends)
                                                  .map(|((_, start), (_, end))| {
                                                      s[*start..*end].to_string()
                                                  })
                                                  .collect();

                    let part = part.join("");

                    if part.len() > 0 {
                        // Remove any extra whitespace outside the quotes
                        let part = &part[..].trim();
                        // Replace quoted characters
                        let part = part.replace("\\\"", "\"");
                        let part = part.replace("\\\'", "\'");
                        let part = part.replace("\\ ", " ");
                        let part = part.replace("\\\\", "\\");
                        parts.push(part);
                    }
                }

                positions.clear();
                positions.push(i + 1);
            }
            (_, _) => (),
        }
        last = c;
    }
    parts
}

trait MacroArgsVisitor {
    fn visit_str(&mut self, name: Option<&str>, val: &str) -> bool;
    fn visit_int(&mut self, name: Option<&str>, val: i64) -> bool;
    fn visit_uint(&mut self, name: Option<&str>, val: u64) -> bool;
    fn visit_bool(&mut self, name: Option<&str>, val: bool) -> bool;
    fn visit_ident(&mut self, name: Option<&str>, ident: &str) -> bool;
}

struct BindgenArgsVisitor<'a> {
    options: &'a mut BindgenOptions,
    seen_named: bool,
}

impl<'a> MacroArgsVisitor for BindgenArgsVisitor<'a> {
    fn visit_str(&mut self, mut name: Option<&str>, val: &str) -> bool {
        if name.is_some() {
            self.seen_named = true;
        } else if !self.seen_named {
            name = Some("clang_args")
        }
        match name {
            Some("link") => {
                let parts = val.split('=').collect::<Vec<_>>();
                self.options.links.push(match parts.len() {
                    1 => (parts[0].to_string(), LinkType::Dynamic),
                    2 => {
                        (parts[1].to_string(),
                         match parts[0] {
                            "static" => LinkType::Static,
                            "dynamic" => LinkType::Dynamic,
                            "framework" => LinkType::Framework,
                            _ => return false,
                        })
                    }
                    _ => return false,
                })
            }
            Some("match") => self.options.match_pat.push(val.to_string()),
            Some("clang_args") => self.options.clang_args.extend(parse_process_args(val)),
            Some("enum_type") => self.options.override_enum_ty = val.to_string(),
            _ => return false,
        }
        true
    }

    fn visit_int(&mut self, name: Option<&str>, _val: i64) -> bool {
        if name.is_some() {
            self.seen_named = true;
        }
        false
    }

    fn visit_uint(&mut self, name: Option<&str>, _val: u64) -> bool {
        if name.is_some() {
            self.seen_named = true;
        }
        false
    }

    fn visit_bool(&mut self, name: Option<&str>, val: bool) -> bool {
        if name.is_some() {
            self.seen_named = true;
        }
        match name {
            Some("allow_unknown_types") => self.options.fail_on_unknown_type = !val,
            Some("builtins") => self.options.builtins = val,
            _ => return false,
        }
        true
    }

    fn visit_ident(&mut self, name: Option<&str>, _val: &str) -> bool {
        if name.is_some() {
            self.seen_named = true;
        }
        false
    }
}


#[test]
fn test_parse_process_args() {
    assert_eq!(parse_process_args("a b c"), vec!["a", "b", "c"]);
    assert_eq!(parse_process_args("a \"b\" c"), vec!["a", "b", "c"]);
    assert_eq!(parse_process_args("a \'b\' c"), vec!["a", "b", "c"]);
    assert_eq!(parse_process_args("a \"b c\""), vec!["a", "b c"]);
    assert_eq!(parse_process_args("a \'\"b\"\' c"), vec!["a", "\"b\"", "c"]);
    assert_eq!(parse_process_args("a b\\ c"), vec!["a", "b c"]);
    assert_eq!(parse_process_args("a b c\\"), vec!["a", "b", "c\\"]);
}
