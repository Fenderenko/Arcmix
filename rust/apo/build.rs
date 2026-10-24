// The COM entry points are exported PRIVATE (as COM servers do), from a module definition file.
fn main() {
    let def = std::path::Path::new(&std::env::var("CARGO_MANIFEST_DIR").unwrap()).join("arcmix_apo.def");
    println!("cargo:rerun-if-changed=arcmix_apo.def");
    println!("cargo:rustc-cdylib-link-arg=/DEF:{}", def.display());
}
