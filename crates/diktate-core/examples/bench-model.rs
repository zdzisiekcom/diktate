//! Diagnostyka kosztu transkrypcji: ładuje model **raz** i transkrybuje ten
//! sam materiał kilka razy, mierząc każdy przebieg osobno.
//!
//! Po co osobno: daemon wczytuje model przy starcie i pracuje z nim przez
//! całą sesję, więc do doboru progów segmentacji liczy się koszt przebiegu
//! *kolejnego*, a nie pierwszego. Pierwszy bywa droższy — backend dopiero
//! alokuje bufory i rozgrzewa potok — a pomiar przez `diktate-cli` tego nie
//! pokazuje, bo każde uruchomienie CLI to nowy proces z jedną transkrypcją.
//!
//! cargo run -p diktate-core --example bench-model -- plik.wav [powtórzenia]
//!
//! Model wskazuje `$DIKTATE_MODEL` albo ścieżka domyślna.

use std::time::Instant;

use diktate_core::transcribe::{default_model_path, Transcriber};

fn main() -> anyhow::Result<()> {
    let path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "testdata/jfk.wav".into());
    let runs: usize = std::env::args()
        .nth(2)
        .and_then(|a| a.parse().ok())
        .unwrap_or(4);

    let mut reader = hound::WavReader::open(&path)?;
    let spec = reader.spec();
    let samples: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Int => reader
            .samples::<i16>()
            .map(|s| s.map(|v| v as f32 / i16::MAX as f32))
            .collect::<Result<_, _>>()?,
        hound::SampleFormat::Float => reader.samples::<f32>().collect::<Result<_, _>>()?,
    };
    let seconds = samples.len() as f32 / spec.sample_rate as f32;

    let model = std::env::var_os("DIKTATE_MODEL")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(default_model_path);

    let loading = Instant::now();
    let transcriber = Transcriber::new(&model)?.with_language("pl".to_string());
    let load_ms = loading.elapsed().as_millis();

    println!("model: {}", model.display());
    println!("materiał: {path} ({seconds:.1} s)");
    println!("wczytanie modelu: {load_ms} ms (daemon płaci to raz, przy starcie)");

    let mut times = Vec::with_capacity(runs);
    for i in 1..=runs {
        let started = Instant::now();
        let text = transcriber.transcribe(&samples, "")?;
        let ms = started.elapsed().as_millis();
        times.push(ms);
        println!("przebieg {i}: {ms} ms{}", if i == 1 { "  (pierwszy)" } else { "" });
        if i == runs {
            println!("tekst: {}", text.trim());
        }
    }

    if let Some((first, rest)) = times.split_first() {
        if !rest.is_empty() {
            let avg: u128 = rest.iter().sum::<u128>() / rest.len() as u128;
            println!("pierwszy: {first} ms, średnia kolejnych: {avg} ms");
            println!(
                "koszt kolejnego przebiegu wobec długości materiału: {:.2}× czasu rzeczywistego",
                avg as f32 / 1000.0 / seconds
            );
        }
    }
    Ok(())
}
