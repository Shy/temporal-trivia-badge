use anyhow::Result;
use esp_idf_svc::hal::{
    delay::BLOCK,
    gpio::{Gpio4, Gpio5},
    i2c::{I2C0, I2cConfig, I2cDriver},
    units::KiloHertz,
};

use crate::model::{GameSnapshot, Question};

const ADDRESS: u8 = 0x3c;
const WIDTH: usize = 128;
const HEIGHT: usize = 64;
const BUFFER_LEN: usize = WIDTH * HEIGHT / 8;

pub struct BadgeDisplay {
    i2c: I2cDriver<'static>,
    buffer: [u8; BUFFER_LEN],
}

impl BadgeDisplay {
    pub fn new(i2c: I2C0<'static>, sda: Gpio4<'static>, scl: Gpio5<'static>) -> Result<Self> {
        let config = I2cConfig::new().baudrate(KiloHertz(400).into());
        let mut display = Self {
            i2c: I2cDriver::new(i2c, sda, scl, &config)?,
            buffer: [0; BUFFER_LEN],
        };
        display.command(&[
            0xae, 0xd5, 0x80, 0xa8, 0x3f, 0xd3, 0x00, 0x40, 0x8d, 0x14, 0x20, 0x00, 0xa1, 0xc8,
            0xda, 0x12, 0x81, 0x50, 0xd9, 0xf1, 0xdb, 0x40, 0xa4, 0xa6, 0xaf,
        ])?;
        Ok(display)
    }

    pub fn show_status(&mut self, title: &str, detail: &str) -> Result<()> {
        self.buffer.fill(0);
        self.draw_text(0, 0, title);
        for x in 0..WIDTH {
            self.set_pixel(x, 9);
        }

        let mut x = 0;
        let mut y = 16;
        for character in detail.chars() {
            if character == '\n' || x + 6 > WIDTH {
                x = 0;
                y += 8;
                if character == '\n' {
                    continue;
                }
            }
            if y + 7 > HEIGHT {
                break;
            }
            self.draw_char(x, y, character);
            x += 6;
        }
        self.flush()
    }

    pub fn show_waiting(&mut self, callsign: &str) -> Result<()> {
        self.show_status(callsign, "WORKER MODE\nWAITING FOR GAME")
    }

    pub fn power_off(&mut self) -> Result<()> {
        self.buffer.fill(0);
        self.flush()?;
        self.command(&[0xae])
    }

    pub fn show_question(&mut self, callsign: &str, question: &Question) -> Result<()> {
        self.buffer.fill(0);
        self.draw_text(0, 0, callsign);
        self.draw_hline(0, 127, 8);
        self.draw_compact_wrapped(1, 11, &question.prompt, 31, 3);

        // Same physical layout as the original badge's button cluster:
        // top/right on row one, left/down on row two.
        for (index, &(x, y)) in [(0, 30), (65, 30), (0, 48), (65, 48)].iter().enumerate() {
            self.draw_frame(x, y, 63, 16);
            self.draw_button_glyph(index, x + 2, y + 3);
            self.draw_compact_wrapped(x + 14, y + 3, &question.answers[index], 11, 2);
        }
        self.flush()
    }

    pub fn show_feedback(&mut self, callsign: &str, correct: bool, score_delta: i32) -> Result<()> {
        self.buffer.fill(0);
        self.draw_text(0, 0, callsign);
        self.draw_hline(0, 127, 9);
        self.draw_text(31, 22, if correct { "CORRECT" } else { "WRONG" });
        self.draw_text(49, 38, if score_delta > 0 { "+1" } else { "-1" });
        self.flush()
    }

    pub fn show_panic(&mut self, callsign: &str) -> Result<()> {
        self.buffer.fill(0);
        self.draw_text(0, 0, callsign);
        self.draw_frame(5, 15, 118, 36);
        self.draw_text(39, 23, "PANIC!");
        self.draw_compact_wrapped(23, 37, "WORKER CRASH SIMULATED", 22, 2);
        self.flush()
    }

    pub fn show_recovered(&mut self, callsign: &str) -> Result<()> {
        self.show_status(callsign, "WORKER RECOVERED\nQUESTION RETURNED")
    }

    pub fn show_results(
        &mut self,
        callsign: &str,
        badge_id: &str,
        snapshot: &GameSnapshot,
    ) -> Result<()> {
        let own = snapshot.players.get(badge_id);
        let own_score = own.map(|player| player.score).unwrap_or(0);
        let place = 1 + snapshot
            .players
            .values()
            .filter(|player| player.score > own_score)
            .count();
        let won = snapshot.winners.iter().any(|winner| winner == callsign);
        self.buffer.fill(0);
        self.draw_text(0, 0, callsign);
        self.draw_hline(0, 127, 9);
        self.draw_text(0, 15, if won { "YOU WON" } else { "ROUND OVER" });
        self.draw_text(0, 27, &format!("SCORE {own_score}"));
        self.draw_text(0, 38, &format!("PLACE {place}"));
        self.draw_compact_wrapped(
            0,
            51,
            &format!("WINNER {}", snapshot.winners.join(" + ")),
            31,
            2,
        );
        self.flush()
    }

    fn command(&mut self, commands: &[u8]) -> Result<()> {
        let mut packet = Vec::with_capacity(commands.len() + 1);
        packet.push(0x00);
        packet.extend_from_slice(commands);
        self.i2c.write(ADDRESS, &packet, BLOCK)?;
        Ok(())
    }

    fn flush(&mut self) -> Result<()> {
        self.command(&[0x21, 0, 127, 0x22, 0, 7])?;
        for chunk in self.buffer.chunks(16) {
            let mut packet = [0_u8; 17];
            packet[0] = 0x40;
            packet[1..].copy_from_slice(chunk);
            self.i2c.write(ADDRESS, &packet, BLOCK)?;
        }
        Ok(())
    }

    fn draw_text(&mut self, mut x: usize, y: usize, text: &str) {
        for character in text.chars() {
            if x + 6 > WIDTH {
                break;
            }
            self.draw_char(x, y, character);
            x += 6;
        }
    }

    fn draw_compact_wrapped(
        &mut self,
        x: usize,
        y: usize,
        text: &str,
        max_chars: usize,
        max_lines: usize,
    ) {
        for (line_index, line) in wrap(text, max_chars, max_lines).iter().enumerate() {
            self.draw_compact_text(x, y + line_index * 6, line);
        }
    }

    fn draw_compact_text(&mut self, mut x: usize, y: usize, text: &str) {
        for character in text.chars() {
            if x + 3 > WIDTH {
                break;
            }
            self.draw_compact_char(x, y, character);
            x += 4;
        }
    }

    fn draw_compact_char(&mut self, x: usize, y: usize, character: char) {
        let source = glyph(character);
        let x_ranges = [(0, 1), (2, 2), (3, 4)];
        let y_ranges = [(0, 1), (2, 2), (3, 3), (4, 5), (6, 6)];
        for (target_x, &(from_x, to_x)) in x_ranges.iter().enumerate() {
            for (target_y, &(from_y, to_y)) in y_ranges.iter().enumerate() {
                let mut on = false;
                for column in from_x..=to_x {
                    for row in from_y..=to_y {
                        on |= source[column] & (1 << row) != 0;
                    }
                }
                if on {
                    self.set_pixel(x + target_x, y + target_y);
                }
            }
        }
    }

    fn draw_hline(&mut self, from_x: usize, to_x: usize, y: usize) {
        for x in from_x..=to_x {
            self.set_pixel(x, y);
        }
    }

    fn draw_frame(&mut self, x: usize, y: usize, width: usize, height: usize) {
        for column in (x + 2)..(x + width - 2) {
            self.set_pixel(column, y);
            self.set_pixel(column, y + height - 1);
        }
        for row in (y + 2)..(y + height - 2) {
            self.set_pixel(x, row);
            self.set_pixel(x + width - 1, row);
        }
        for (dx, dy) in [
            (1, 1),
            (width - 2, 1),
            (1, height - 2),
            (width - 2, height - 2),
        ] {
            self.set_pixel(x + dx, y + dy);
        }
    }

    fn draw_button_glyph(&mut self, answer_index: usize, x: usize, y: usize) {
        let bits = match answer_index {
            0 => &BUTTON_TOP,
            1 => &BUTTON_RIGHT,
            2 => &BUTTON_LEFT,
            _ => &BUTTON_DOWN,
        };
        for row in 0..10 {
            let row_bits = u16::from(bits[row * 2]) | (u16::from(bits[row * 2 + 1]) << 8);
            for column in 0..10 {
                if row_bits & (1 << column) != 0 {
                    self.set_pixel(x + column, y + row);
                }
            }
        }
    }

    fn draw_char(&mut self, x: usize, y: usize, character: char) {
        for (column, bits) in glyph(character).iter().enumerate() {
            for row in 0..7 {
                if bits & (1 << row) != 0 {
                    self.set_pixel(x + column, y + row);
                }
            }
        }
    }

    fn set_pixel(&mut self, x: usize, y: usize) {
        if x < WIDTH && y < HEIGHT {
            self.buffer[x + (y / 8) * WIDTH] |= 1 << (y % 8);
        }
    }
}

fn wrap(text: &str, max_chars: usize, max_lines: usize) -> Vec<String> {
    let mut lines = Vec::new();
    let mut current = String::new();
    for word in text.split_whitespace() {
        let needed =
            current.chars().count() + usize::from(!current.is_empty()) + word.chars().count();
        if needed > max_chars && !current.is_empty() {
            lines.push(current);
            current = String::new();
            if lines.len() == max_lines {
                return lines;
            }
        }
        if !current.is_empty() {
            current.push(' ');
        }
        current.extend(word.chars().take(max_chars));
    }
    if !current.is_empty() && lines.len() < max_lines {
        lines.push(current);
    }
    lines
}

const BUTTON_TOP: [u8; 20] = [
    0x30, 0x00, 0x78, 0x00, 0x78, 0x00, 0xB6, 0x01, 0x49, 0x02, 0x49, 0x02, 0xB6, 0x01, 0x48, 0x00,
    0x48, 0x00, 0x30, 0x00,
];
const BUTTON_RIGHT: [u8; 20] = [
    0x30, 0x00, 0x48, 0x00, 0x48, 0x00, 0xB6, 0x01, 0xC9, 0x03, 0xC9, 0x03, 0xB6, 0x01, 0x48, 0x00,
    0x48, 0x00, 0x30, 0x00,
];
const BUTTON_DOWN: [u8; 20] = [
    0x30, 0x00, 0x48, 0x00, 0x48, 0x00, 0xB6, 0x01, 0x49, 0x02, 0x49, 0x02, 0xB6, 0x01, 0x78, 0x00,
    0x78, 0x00, 0x30, 0x00,
];
const BUTTON_LEFT: [u8; 20] = [
    0x30, 0x00, 0x48, 0x00, 0x48, 0x00, 0xB6, 0x01, 0x4F, 0x02, 0x4F, 0x02, 0xB6, 0x01, 0x48, 0x00,
    0x48, 0x00, 0x30, 0x00,
];

fn glyph(character: char) -> [u8; 5] {
    match character.to_ascii_uppercase() {
        'A' => [0x7e, 0x11, 0x11, 0x11, 0x7e],
        'B' => [0x7f, 0x49, 0x49, 0x49, 0x36],
        'C' => [0x3e, 0x41, 0x41, 0x41, 0x22],
        'D' => [0x7f, 0x41, 0x41, 0x22, 0x1c],
        'E' => [0x7f, 0x49, 0x49, 0x49, 0x41],
        'F' => [0x7f, 0x09, 0x09, 0x09, 0x01],
        'G' => [0x3e, 0x41, 0x49, 0x49, 0x7a],
        'H' => [0x7f, 0x08, 0x08, 0x08, 0x7f],
        'I' => [0x00, 0x41, 0x7f, 0x41, 0x00],
        'J' => [0x20, 0x40, 0x41, 0x3f, 0x01],
        'K' => [0x7f, 0x08, 0x14, 0x22, 0x41],
        'L' => [0x7f, 0x40, 0x40, 0x40, 0x40],
        'M' => [0x7f, 0x02, 0x0c, 0x02, 0x7f],
        'N' => [0x7f, 0x04, 0x08, 0x10, 0x7f],
        'O' => [0x3e, 0x41, 0x41, 0x41, 0x3e],
        'P' => [0x7f, 0x09, 0x09, 0x09, 0x06],
        'Q' => [0x3e, 0x41, 0x51, 0x21, 0x5e],
        'R' => [0x7f, 0x09, 0x19, 0x29, 0x46],
        'S' => [0x46, 0x49, 0x49, 0x49, 0x31],
        'T' => [0x01, 0x01, 0x7f, 0x01, 0x01],
        'U' => [0x3f, 0x40, 0x40, 0x40, 0x3f],
        'V' => [0x1f, 0x20, 0x40, 0x20, 0x1f],
        'W' => [0x3f, 0x40, 0x38, 0x40, 0x3f],
        'X' => [0x63, 0x14, 0x08, 0x14, 0x63],
        'Y' => [0x07, 0x08, 0x70, 0x08, 0x07],
        'Z' => [0x61, 0x51, 0x49, 0x45, 0x43],
        '0' => [0x3e, 0x51, 0x49, 0x45, 0x3e],
        '1' => [0x00, 0x42, 0x7f, 0x40, 0x00],
        '2' => [0x42, 0x61, 0x51, 0x49, 0x46],
        '3' => [0x21, 0x41, 0x45, 0x4b, 0x31],
        '4' => [0x18, 0x14, 0x12, 0x7f, 0x10],
        '5' => [0x27, 0x45, 0x45, 0x45, 0x39],
        '6' => [0x3c, 0x4a, 0x49, 0x49, 0x30],
        '7' => [0x01, 0x71, 0x09, 0x05, 0x03],
        '8' => [0x36, 0x49, 0x49, 0x49, 0x36],
        '9' => [0x06, 0x49, 0x49, 0x29, 0x1e],
        '-' => [0x08, 0x08, 0x08, 0x08, 0x08],
        '_' => [0x40, 0x40, 0x40, 0x40, 0x40],
        '.' => [0x00, 0x60, 0x60, 0x00, 0x00],
        ':' => [0x00, 0x36, 0x36, 0x00, 0x00],
        '/' => [0x20, 0x10, 0x08, 0x04, 0x02],
        ' ' => [0; 5],
        _ => [0x02, 0x01, 0x51, 0x09, 0x06],
    }
}
