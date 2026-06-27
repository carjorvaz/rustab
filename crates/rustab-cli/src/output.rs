use std::io::{self, Write};

pub(crate) enum TsvField<'a> {
    Id(&'a str),
    Text(&'a str),
}

impl<'a> TsvField<'a> {
    pub(crate) fn id(value: &'a str) -> Self {
        Self::Id(value)
    }

    pub(crate) fn text(value: &'a str) -> Self {
        Self::Text(value)
    }
}

pub(crate) fn write_tsv_row<'a, W, I>(writer: &mut W, fields: I) -> io::Result<()>
where
    W: Write,
    I: IntoIterator<Item = TsvField<'a>>,
{
    let mut first = true;
    for field in fields {
        if first {
            first = false;
        } else {
            writer.write_all(b"\t")?;
        }

        match field {
            TsvField::Id(value) => writer.write_all(value.as_bytes())?,
            TsvField::Text(value) => write_tsv_text_field(writer, value)?,
        }
    }

    writer.write_all(b"\n")
}

fn write_tsv_text_field<W: Write>(writer: &mut W, value: &str) -> io::Result<()> {
    let mut encoded = [0; 4];
    for character in value.chars() {
        if matches!(character, '\r' | '\n' | '\t') {
            writer.write_all(b" ")?;
        } else {
            writer.write_all(character.encode_utf8(&mut encoded).as_bytes())?;
        }
    }
    Ok(())
}

pub fn print_json<T: serde::Serialize>(value: &T) -> Result<(), String> {
    let rendered =
        serde_json::to_string_pretty(value).map_err(|e| format!("failed to render JSON: {e}"))?;
    println!("{rendered}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tsv_row_normalizes_text_fields_to_one_line() {
        let mut output = Vec::new();

        write_tsv_row(
            &mut output,
            [
                TsvField::id("b.7.9"),
                TsvField::text("title\nwith\ttab"),
                TsvField::text("https://example.test/\rpath\tquery"),
            ],
        )
        .unwrap();

        let output = String::from_utf8(output).unwrap();
        assert_eq!(
            output,
            "b.7.9\ttitle with tab\thttps://example.test/ path query\n"
        );
        assert_eq!(output.lines().count(), 1);
    }
}
