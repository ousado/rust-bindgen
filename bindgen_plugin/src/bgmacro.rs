use std::default::Default;
use std::fmt;
use std::path::Path;

use syntax::ast;
use syntax::codemap;
use syntax::ext::base;
use syntax::parse;
use syntax::ptr::P;
use syntax::util::small_vector::SmallVector;

use bindgen::{BindgenOptions, Bindings, Logger};

use clang_sys;

use parser;

pub fn bindgen_macro(cx: &mut base::ExtCtxt,
                     sp: codemap::Span,
                     tts: &[ast::TokenTree])
                     -> Box<base::MacResult + 'static> {
     let mut options = BindgenOptions {
         builtins: true,
         ..Default::default()
     };

    if !parser::parse_macro_opts(cx, tts, &mut options) {
        return base::DummyResult::any(sp);
    }

    let clang = clang_sys::support::Clang::find(None).expect("No clang found, is it installed?");
    for dir in clang.c_search_paths {
        options.clang_args.push("-idirafter".to_owned());
        options.clang_args.push(dir.to_str().unwrap().to_owned());
    }

    // Add the directory of the header to the include search path.
    let filename = cx.codemap().span_to_filename(sp);
    let mod_dir = Path::new(&filename).parent().unwrap();
    options.clang_args.push("-I".into());
    options.clang_args.push(mod_dir.to_str().unwrap().into());

    // We want the span for errors to just match the bindgen! symbol
    // instead of the whole invocation which can span multiple lines
    let mut short_span = sp;
    short_span.hi = short_span.lo + codemap::BytePos(8);

    let logger = MacroLogger {
        sp: short_span,
        cx: cx,
    };

    match Bindings::generate(&options, Some(&logger as &Logger), None) {
        Ok(bindings) => {
            // syntex_syntax is not compatible with libsyntax so convert to string and reparse
            let bindings_str = bindings.to_string();
            // Unfortunately we lose span information due to reparsing
            let mut parser = parse::new_parser_from_source_str(cx.parse_sess(),
                                                               cx.cfg(),
                                                               "(Auto-generated bindings)"
                                                                   .to_string(),
                                                               bindings_str);

            let mut items = Vec::new();
            while let Ok(Some(item)) = parser.parse_item() {
                items.push(item);
            }

            Box::new(BindgenResult {
                items: Some(SmallVector::many(items)),
            }) as Box<base::MacResult>

        }
        Err(_) => base::DummyResult::any(sp),
    }
}

struct MacroLogger<'a, 'b: 'a> {
    sp: codemap::Span,
    cx: &'a base::ExtCtxt<'b>,
}

impl<'a, 'b> Logger for MacroLogger<'a, 'b> {
    fn error(&self, msg: &str) {
        self.cx.span_err(self.sp, msg)
    }

    fn warn(&self, msg: &str) {
        self.cx.span_warn(self.sp, msg)
    }
}

impl<'a, 'b> fmt::Debug for MacroLogger<'a, 'b> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "MacroLogger")
    }
}

struct BindgenResult {
    items: Option<SmallVector<P<ast::Item>>>,
}

impl base::MacResult for BindgenResult {
    fn make_items(mut self: Box<BindgenResult>) -> Option<SmallVector<P<ast::Item>>> {
        self.items.take()
    }
}
