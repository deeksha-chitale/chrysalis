use chrysalis::sereal::{header, body, Cursor};
use chrysalis::sereal::value::SerealSeenTable;
use chrysalis::storable::value::PerlValue;

#[test]
fn parses_sereal_specimen() {
    let bytes = std::fs::read("sereal_specimen.bin")
        .expect("run tools/sereal_specimen.pl first");

    let (hdr, remaining) = header::parse(&bytes).expect("header should parse");
    println!("Sereal version {} encoding {:?}", hdr.version, hdr.encoding);

    let mut cursor = Cursor::new(remaining);
    let mut seen = SerealSeenTable::new();
    let val = body::read_value(&mut cursor, &mut seen).expect("body should parse");

    // sereal_specimen.bin encodes { answer => 42 }
    match &*val.borrow() {
        PerlValue::Ref(inner) => match &*inner.borrow() {
            PerlValue::Hash(map) => {
                let v = map.get(b"answer".as_slice()).expect("answer key");
                assert!(matches!(*v.borrow(), PerlValue::Integer(42)));
            }
            other => panic!("expected Hash inside Ref: {:?}", other),
        },
        PerlValue::Hash(map) => {
            let v = map.get(b"answer".as_slice()).expect("answer key");
            assert!(matches!(*v.borrow(), PerlValue::Integer(42)));
        }
        other => panic!("expected Hash or Ref(Hash): {:?}", other),
    }
}