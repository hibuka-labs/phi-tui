//! `Picker<T>` + `picker_key` demo: the shared `@`/`/` completion state machine
//! driven over plain stdin — no terminal setup, easy to try anywhere.
//!
//! Run: `cargo run --example picker-demo`
//! Keys: type to filter, Up/Down (or j/k) to move, Enter to confirm, Esc to cancel.

use std::io::{self, BufRead, Write};

use phi_tui::picker::{Picker, PickerKey};

fn main() -> io::Result<()> {
    let skills: Vec<String> = vec!["review", "ship", "spec", "qa", "investigate"]
        .into_iter()
        .map(str::to_string)
        .collect();
    let mut picker: Picker<String> = Picker::new();
    picker.entries = skills.clone();

    println!("pick a skill (filter / arrows j,k / Enter confirm / Esc cancel):");
    let stdin = io::stdin();
    for line in stdin.lock().lines() {
        let line = line?;
        // Drive the state machine through `picker_key` — the same mapping the
        // TUI uses for real KeyCode events.
        for c in line.chars() {
            let key = match c {
                'j' => PickerKey::Move(1),
                'k' => PickerKey::Move(-1),
                other => PickerKey::Char(other),
            };
            apply(&mut picker, key);
        }
        picker.entries = skills
            .iter()
            .filter(|s| s.starts_with(&picker.prefix))
            .map(|s| s.to_string())
            .collect();
        picker.clamp_selection();
        render(&picker);
        if picker.prefix == "quit" {
            break;
        }
    }
    Ok(())
}

fn apply(picker: &mut Picker<String>, key: PickerKey) {
    match key {
        PickerKey::Char(c) => picker.push_char(c),
        PickerKey::Backspace => picker.pop_char(),
        PickerKey::Move(delta) => picker.move_selection(delta),
        PickerKey::Confirm | PickerKey::Cancel => {}
    }
}

fn render(picker: &Picker<String>) {
    let mut out = String::new();
    for (i, entry) in picker.entries.iter().enumerate() {
        let marker = if i == picker.selected { ">" } else { " " };
        out.push_str(&format!("{marker} {entry}\n"));
    }
    out.push_str(&format!(
        "prefix: {:?} — Enter on your choice\n",
        picker.prefix
    ));
    print!("\x1B[2J\x1B[H{out}");
    let _ = io::stdout().flush();
}
