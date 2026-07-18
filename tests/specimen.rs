use chrysalis::storable::{header, body};
use chrysalis::storable::value::{PerlValue, SeenTable, ClassTable};

#[test]
fn parses_specimen() {
    let bytes = std::fs::read("specimen.bin").expect("run tools/specimen.pl first");
    let (h, remaining) = header::parse(&bytes).expect("header should parse");
    let config = h.body_config();

    let mut cursor = body::Cursor::new(remaining);
    let mut seen = SeenTable::new();
    let mut classes = ClassTable::new();
    let val = body::read_value(&mut cursor, &mut seen, &mut classes, &config)
        .expect("body should parse");

    // specimen.bin serializes { "answer" => 42 }
    match &*val.borrow() {
        PerlValue::Hash(map) => {
            assert_eq!(map.len(), 1);
            let v = map.get(b"answer".as_slice()).expect("key 'answer' missing");
            assert!(matches!(*v.borrow(), PerlValue::Integer(42)));
        }
        other => panic!("expected Hash, got {:?}", other),
    }
}