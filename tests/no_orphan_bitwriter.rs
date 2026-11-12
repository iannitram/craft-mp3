//! Locked tests for issue #2: the orphaned src/bitwriter.rs is gone.

use std::fs;
use std::path::Path;

fn manifest() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

#[test]
fn bitwriter_file_is_deleted() {
    assert!(!manifest().join("src").join("bitwriter.rs").exists());
}

#[test]
fn src_holds_exactly_the_expected_files() {
    let mut names: Vec<String> = fs::read_dir(manifest().join("src"))
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    names.sort();
    assert_eq!(
        names,
        [
            "attack.rs",
            "bitstream.rs",
            "encoder.rs",
            "filterbank.rs",
            "header.rs",
            "huffman.rs",
            "huffman_tables.rs",
            "id3.rs",
            "info.rs",
            "layout.rs",
            "lib.rs",
            "mdct.rs",
            "psycho.rs",
            "quantize.rs",
            "reservoir.rs",
            "tables.rs",
        ]
    );
}

#[test]
fn lib_rs_does_not_declare_a_bitwriter_module() {
    let lib = fs::read_to_string(manifest().join("src").join("lib.rs")).unwrap();
    assert!(!lib.contains("mod bitwriter"));
}
