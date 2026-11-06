//! Writes a realistic 26-hour log for a fake checkout service, ending now:
//!   - v1.4.1 runs normally (requests, occasional slow queries and timeouts)
//!   - v1.4.2 deploys 50 minutes ago: a new KeyError crash and a timeout spike
//!   - v1.4.3 deploys 15 minutes ago with the fix
//!
//! Lines include emails and tokens so you can see redaction work.
//!
//!   cargo run --example demo_log -- demo.log

use std::io::Write;

fn iso(ts: f64) -> String {
    let secs = ts.floor() as i64;
    let ms = ((ts - ts.floor()) * 1000.0) as i64;
    let (d, s) = (secs.div_euclid(86400), secs.rem_euclid(86400));
    // Civil date from days since epoch (Howard Hinnant's algorithm).
    let z = d + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.{ms:03}Z", s / 3600, s % 3600 / 60, s % 60)
}

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

fn main() {
    let path = std::env::args().nth(1).unwrap_or_else(|| "demo.log".into());
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs_f64();
    let start = now - 26.0 * 3600.0;
    let bad_deploy = now - 50.0 * 60.0;
    let fix_deploy = now - 15.0 * 60.0;
    let mut r = Rng(0x9e3779b97f4a7c15);
    let mut out = std::io::BufWriter::new(std::fs::File::create(&path).unwrap());
    let users = ["alice@example.com", "bob@example.org", "chen@example.net"];

    writeln!(out, "{} INFO Starting checkout-service version 1.4.1 on port 8080", iso(start)).unwrap();
    let mut t = start + 1.0;
    let (mut v142, mut v143) = (false, false);
    while t < now {
        if !v142 && t >= bad_deploy {
            writeln!(out, "{} INFO Starting checkout-service version 1.4.2 on port 8080", iso(t)).unwrap();
            v142 = true;
        }
        if !v143 && t >= fix_deploy {
            writeln!(out, "{} INFO Starting checkout-service version 1.4.3 on port 8080", iso(t)).unwrap();
            v143 = true;
        }
        let buggy = v142 && !v143;
        let ts = iso(t);
        let user = users[r.below(3) as usize];
        let cart = 1000 + r.below(9000);
        match r.below(100) {
            0..=69 => writeln!(out, "{ts} INFO GET /api/cart/{cart} 200 in {}ms user={user}", 5 + r.below(80)),
            70..=84 => writeln!(out, "{ts} INFO POST /api/checkout 201 in {}ms order=ord_{:x}", 40 + r.below(200), r.next() % 0xffffff),
            85..=91 => writeln!(out, "{ts} WARN slow query on carts took {}ms", 500 + r.below(1500)),
            92..=93 => writeln!(out, "{ts} INFO auth refreshed for {user} Authorization: Bearer tok_{:x}{:x}", r.next(), r.next()),
            94 if buggy || r.below(12) == 0 => writeln!(out, "{ts} ERROR upstream payments timeout after 3000ms (attempt {})", 1 + r.below(3)),
            _ if buggy && r.below(4) < 3 => writeln!(
                out,
                "{ts} ERROR Unhandled error in POST /api/checkout cart={cart}\nTraceback (most recent call last):\n  File \"/app/checkout.py\", line 88, in place_order\n    total = price_cart(cart)\n  File \"/app/pricing.py\", line 41, in price_cart\n    currency = cart.meta['currency']\nKeyError: 'currency'"
            ),
            _ => writeln!(out, "{ts} DEBUG cache hit cart:{cart}"),
        }
        .unwrap();
        // Busier during the bad deploy because clients retry.
        t += if buggy { 2.0 + r.below(4) as f64 } else { 10.0 + r.below(40) as f64 };
    }
    eprintln!("wrote {path}");
}
