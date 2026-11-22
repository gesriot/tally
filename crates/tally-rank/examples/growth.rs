//! Рост длины ключей при вставках в одно место: `cargo run -p tally-rank --example growth`.

use tally_rank::Rank;

fn report(name: &str, keys: &[Rank]) {
    let max = keys.iter().map(|k| k.as_str().len()).max().unwrap_or(0);
    let avg = keys.iter().map(|k| k.as_str().len()).sum::<usize>() as f64 / keys.len() as f64;
    println!("{name:<38} max {max:>5}  средн. {avg:>7.1}");
}

fn main() {
    for n in [100usize, 1_000, 10_000] {
        println!("--- {n} вставок ---");
        let mut end = vec![Rank::initial()];
        for _ in 0..n {
            let k = end[end.len() - 1].after();
            end.push(k);
        }
        report("в конец", &end);

        let mut front = vec![Rank::initial()];
        for _ in 0..n {
            let k = front[0].before();
            front.insert(0, k);
        }
        report("в начало", &front);

        let mut mid = vec![Rank::initial(), Rank::initial().after()];
        for _ in 0..n {
            let k = Rank::between(Some(&mid[0]), Some(&mid[1]));
            mid.insert(1, k);
        }
        report("всегда сразу после одного соседа", &mid);

        let mut alt = vec![Rank::initial(), Rank::initial().after()];
        for i in 0..n {
            // Чередуем: в середину текущего промежутка между двумя «старыми» соседями.
            let j = 1 + (i % 2) * ((alt.len() - 1) / 2);
            let k = Rank::between(Some(&alt[j - 1]), Some(&alt[j]));
            alt.insert(j, k);
        }
        report("в середину списка (чередуя)", &alt);

        report("ребалансировка spread(n)", &Rank::spread(n));
    }
}
