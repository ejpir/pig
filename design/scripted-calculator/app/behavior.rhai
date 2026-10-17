// Loaded at runtime, never compiled to Rust. Only dispatch is called.
// Immediate-execution calculator: 2 + 3 × 4 = 20. Percent divides by 100.
// Numeric results retain full precision; only their presentation is formatted.

fn reset(s) {
    s.entry = "0";
    s.display = "0";
    s.value = 0.0;
    s.lhs = 0.0;
    s.op = "";
    s.mode = "entry";
    s.caption = "";
    s
}

fn symbol(op) {
    if op == "*" { "×" }
    else if op == "/" { "÷" }
    else if op == "-" { "−" }
    else { op }
}

fn arithmetic(a, b, op) {
    if op == "/" && b == 0.0 { throw "Cannot divide by zero"; }
    let result = if op == "+" { a + b }
        else if op == "-" { a - b }
        else if op == "*" { a * b }
        else { a / b };
    if !finite(result) { throw "Result out of range"; }
    result
}

fn refresh(s) {
    s.display = if s.mode == "entry" { s.entry } else { fmt(s.value) };
    s
}

fn dispatch(s, key) {
    let digits = ["0", "1", "2", "3", "4", "5", "6", "7", "8", "9"];
    let operators = ["+", "-", "*", "/"];
    if !(key in digits) && !(key in operators)
        && !(key in [".", "=", "AC", "sign", "%", "backspace"]) { return s; }
    if key == "AC" { return reset(s); }
    if s.mode == "error" { s = reset(s); }

    try {
        if key in digits || key == "." {
            if s.mode != "entry" {
                s.entry = "0";
                s.value = 0.0;
                s.mode = "entry";
                if s.op == "" { s.caption = ""; }
            }
            if key == "." {
                if !s.entry.contains(".") { s.entry += "."; }
            } else if s.entry.len() < 15 {
                if s.entry == "0" { s.entry = key; }
                else if s.entry == "-0" { s.entry = "-" + key; }
                else { s.entry += key; }
            }
            s.value = parse_float(s.entry);
        } else if key == "backspace" {
            if s.mode == "entry" {
                s.entry = s.entry.sub_string(0, s.entry.len() - 1);
                if s.entry == "" || s.entry == "-" { s.entry = "0"; }
                s.value = parse_float(s.entry);
            }
        } else if key == "sign" {
            if s.mode == "pending" {
                s.entry = "-0";
                s.value = -0.0;
                s.mode = "entry";
            } else if s.mode == "entry" {
                if s.entry.starts_with("-") { s.entry = s.entry.sub_string(1); }
                else { s.entry = "-" + s.entry; }
                s.value = parse_float(s.entry);
            } else { s.value = -s.value; }
        } else if key == "%" {
            if s.mode == "pending" { s.value = 0.0; }
            s.value /= 100.0;
            s.mode = "result";
        } else if key in operators {
            if s.op != "" && s.mode != "pending" {
                s.value = arithmetic(s.lhs, s.value, s.op);
            }
            s.lhs = s.value;
            s.op = key;
            s.mode = "pending";
            s.caption = fmt(s.value) + " " + symbol(key);
        } else if key == "=" {
            if s.op == "" || s.mode == "pending" { return s; }
            let rhs = s.value;
            s.value = arithmetic(s.lhs, rhs, s.op);
            s.caption = fmt(s.lhs) + " " + symbol(s.op) + " " + fmt(rhs) + " =";
            s.history3 = s.history2;
            s.history2 = if s.history1 == "No calculations yet" { "" } else { s.history1 };
            s.history1 = fmt(s.lhs) + " " + symbol(s.op) + " " + fmt(rhs) + " = " + fmt(s.value);
            s.op = "";
            s.mode = "result";
        }
        return refresh(s);
    } catch (message) {
        s = reset(s);
        s.mode = "error";
        s.display = "Error";
        s.caption = message.to_string();
        return s;
    }
}
