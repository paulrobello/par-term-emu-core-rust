// Rectangle operations (DECFRA, DECCRA, etc.)
use crate::terminal::*;

#[test]
fn test_decfra_fill_rectangle() {
    let mut term = Terminal::new(80, 24);
    term.process(b"\x1b[88;3;3;5;5$x");

    for row in 2..=4 {
        for col in 2..=4 {
            assert_eq!(term.grid().get(col, row).unwrap().c, 'X');
        }
    }
}
