use super::*;

const FW_XYZ: &[char] = &['Ｘ', 'Ｙ', 'Ｚ'];

#[test]
fn takes() -> io::Result<()> {
    let mut rdr = Utf8Reader::from_str("Ｘ01234567Ｙ89abcdefＺ");

    assert_eq!(rdr.take_once(FW_XYZ)?, Some(FW_XYZ[0]));
    assert_eq!(rdr.take_times(digit, 2)?, Ok(("01", Some('2'))));
    assert_eq!(rdr.take_while(all!(octdigit, FW_XYZ))?, ("234567Ｙ", Some('8')));

    assert_eq!(rdr.take_times(not!(alphabetic), 3..)?, (Err(("89", Some('a')))));

    assert_eq!(rdr.take_while(any)?, ("89abcdefＺ", None));

    assert!(rdr.exhausted());

    Ok(())
}

token_set! {
    Word {
        FOO = "Foo";
        BAR = "Bar";
        BAZ = "Baz";
        HELLO = "你好";
    }
}

#[test]
fn matches() -> io::Result<()> {
    assert_eq!(Utf8Reader::from_str("Baz").matches(WordTokens)?, Some(WordToken::BAZ));
    assert_eq!(
        Utf8Reader::from_str("你好").matches(WordTokens)?,
        Some(WordToken::HELLO)
    );
    assert_eq!(Utf8Reader::from_str("Boom").matches(WordTokens)?, None);

    Ok(())
}

#[test]
fn until() -> io::Result<()> {
    let mut rdr = Utf8Reader::from_str("~Boom~Baz~Bar~Foo~你好~");

    assert_eq!(rdr.skim_until(WordTokens)?, Ok(("~Boom~", WordToken::BAZ)));
    assert_eq!(rdr.skim_until(WordTokens)?, Ok(("~", WordToken::BAR)));
    assert_eq!(rdr.skim_until(WordTokens)?, Ok(("~", WordToken::FOO)));
    assert_eq!(rdr.skim_until(WordTokens)?, Ok(("~", WordToken::HELLO)));
    assert_eq!(rdr.skim_until(WordTokens)?, Err("~"));

    Ok(())
}
