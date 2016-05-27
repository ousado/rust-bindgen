#![feature(plugin)]
#![plugin(bindgen_plugin)]

mod mysql_basic {
    bindgen!("header/basic.h");
}
