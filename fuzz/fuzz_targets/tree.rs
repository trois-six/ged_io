//! The decoder and the lexer are total and consistent: for any bytes,
//! decoding never panics and gives the same text in memory and streaming
//! (whatever the buffer size), the lossless tree reads its own dump back
//! unchanged, and streaming yields the tree's records, line numbers included.
#![no_main]

use std::io::{BufReader, Read};

use ged_io::encoding::{decode, DecodeReader};
use ged_io::tree::{parse_tree, Structure, Tree, TreeReader};
use libfuzzer_sys::fuzz_target;

fn lines(s: &Structure, out: &mut Vec<u32>) {
    out.push(s.line);
    for c in &s.substructures {
        lines(c, out);
    }
}

fuzz_target!(|data: &[u8]| {
    // The first byte picks a buffer size for the streaming paths.
    let (capacity, bytes) = match data.split_first() {
        Some((&c, rest)) => (usize::from(c % 32) + 1, rest),
        None => (1, data),
    };

    let decoded = decode(bytes);
    let mut streamed = String::new();
    DecodeReader::new(BufReader::with_capacity(capacity, bytes))
        .expect("reading a slice cannot fail")
        .read_to_string(&mut streamed)
        .expect("decoded text is UTF-8");
    assert_eq!(streamed, decoded.text);

    let tree = Tree::parse(decoded.text);
    let records = tree.to_structures();
    assert_eq!(parse_tree(&tree.to_gedcom()).to_structures(), records);

    let reader: Vec<Structure> = TreeReader::new(BufReader::with_capacity(capacity, bytes))
        .expect("reading a slice cannot fail")
        .collect::<Result<_, _>>()
        .expect("reading a slice cannot fail");
    assert_eq!(reader, records);
    let (mut a, mut b) = (Vec::new(), Vec::new());
    reader.iter().for_each(|s| lines(s, &mut a));
    records.iter().for_each(|s| lines(s, &mut b));
    assert_eq!(a, b);
});
