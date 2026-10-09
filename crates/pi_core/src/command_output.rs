use std::borrow::Cow;

/// Shell output as a compact log snapshot. CRLF is one newline; text after a
/// bare carriage return replaces the current logical line. A trailing carriage
/// return leaves that line visible until more output arrives.
pub fn for_display(raw: &str) -> Cow<'_, str> {
    if !raw.contains('\r') {
        return Cow::Borrowed(raw);
    }

    let mut displayed = String::with_capacity(raw.len());
    let mut line_start = 0;
    let mut pending_carriage_return = false;

    for character in raw.chars() {
        if pending_carriage_return {
            match character {
                '\n' => {
                    displayed.push('\n');
                    line_start = displayed.len();
                    pending_carriage_return = false;
                    continue;
                }
                '\r' => continue,
                _ => {
                    displayed.truncate(line_start);
                    pending_carriage_return = false;
                }
            }
        }

        match character {
            '\r' => pending_carriage_return = true,
            '\n' => {
                displayed.push('\n');
                line_start = displayed.len();
            }
            _ => displayed.push(character),
        }
    }

    Cow::Owned(displayed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_output_is_compacted_for_log_display() {
        let plain = "plain\ntext";
        assert!(
            matches!(for_display(plain), Cow::Borrowed(text) if text.as_ptr() == plain.as_ptr())
        );
        assert_eq!(for_display("one\r\ntwo"), "one\ntwo");
        assert_eq!(for_display("10%\r50%\r100%\nDone"), "100%\nDone");
        assert_eq!(for_display("before\nlong\rshort"), "before\nshort");
        assert_eq!(for_display("still visible\r"), "still visible");
        assert_eq!(for_display("kept\r\r\nnext"), "kept\nnext");
        assert_eq!(for_display("猫猫\r犬"), "犬");
    }
}
