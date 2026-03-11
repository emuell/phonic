//! Interactive example that lets you pick an audio driver, device, sample rate and buffer size
//! from the command line using console menus, then plays a given audio file.
//!
//! Usage: `cargo run --example play-device --features cpal-output -- <audio-file>`

use std::{
    env,
    io::{self, BufRead, Write},
    sync::mpsc::sync_channel,
    time::Duration,
};

use phonic::{
    outputs::{CpalOutput, CpalOutputConfig, CpalOutputDeviceDriver},
    Error, FilePlaybackOptions, PlaybackStatusEvent, Player,
};

// -------------------------------------------------------------------------------------------------

// Common example code
mod common;
use common::arguments;

// -------------------------------------------------------------------------------------------------

fn select_item<T: Clone>(title: &str, items: &[T], display: impl Fn(&T) -> String) -> Option<T> {
    if items.is_empty() {
        println!("  (no options available)");
        return None;
    }
    println!("\n{title}:");
    for (i, item) in items.iter().enumerate() {
        println!("  {}: {}", i + 1, display(item));
    }
    let stdin = io::stdin();
    loop {
        print!("Select [1-{}] (or Enter for 'Default'): ", items.len());
        io::stdout().flush().ok();
        let line = match stdin.lock().lines().next() {
            Some(Ok(l)) => l,
            _ => return None,
        };
        let line = line.trim().to_owned();
        if line.is_empty() {
            return Some(items[0].clone());
        }
        match line.parse::<usize>() {
            Ok(idx) if idx >= 1 && idx <= items.len() => return Some(items[idx - 1].clone()),
            _ => println!(
                "  Invalid choice, please enter a number between 1 and {}.",
                items.len()
            ),
        }
    }
}

// -------------------------------------------------------------------------------------------------

fn main() -> Result<(), Error> {
    arguments::create_logger(None);

    let file_path = env::args().nth(1).unwrap_or_else(|| {
        eprintln!("Usage: play-device <audio-file>");
        std::process::exit(1);
    });

    // Select audio host/driver
    let drivers = CpalOutput::available_drivers();
    let driver = select_item("Select Audio Driver", &drivers, |h| format!("{h:?}"))
        .unwrap_or(CpalOutputDeviceDriver::Default);
    println!("Driver: {driver:?}");

    // Select output device
    let mut device_entries = vec![(None, "Default".to_string())];
    match CpalOutput::available_devices(driver) {
        Ok(drivers) => device_entries.extend(
            drivers
                .iter()
                .map(|(id, name)| (Some(id.clone()), name.clone())),
        ),
        Err(err) => eprintln!("Warning: could not enumerate devices: {err}"),
    }
    let (device_id, device_name) =
        select_item("Select Output Device", &device_entries, |(_, name)| {
            name.clone()
        })
        .unwrap_or_else(|| (None, "Default".to_string()));
    println!("Device: {}", device_name);

    //  Select sample rate
    let mut rate_entries: Vec<Option<u32>> = vec![None]; // None → auto
    match CpalOutput::supported_sample_rates(driver, device_id.clone()) {
        Ok(rates) => rate_entries.extend(rates.into_iter().map(Some)),
        Err(err) => eprintln!("Warning: could not enumerate sample rates: {err}"),
    }
    let sample_rate = select_item("Select Sample Rate", &rate_entries, |r| {
        r.map(|v| format!("{v} Hz"))
            .unwrap_or_else(|| "Default / Auto".to_string())
    })
    .flatten();
    println!(
        "Sample rate: {}",
        sample_rate
            .map(|r| format!("{r} Hz"))
            .unwrap_or_else(|| "Default".to_string())
    );

    // Select buffer size
    let buffer_sizes: Vec<Option<u32>> = vec![None, Some(256), Some(512), Some(1024), Some(2048)];
    let buffer_size = select_item("Select Buffer Size", &buffer_sizes, |b| {
        b.map(|v| format!("{v} frames"))
            .unwrap_or_else(|| "Default".to_string())
    })
    .flatten();
    println!(
        "Buffer size: {}",
        buffer_size
            .map(|b| format!("{b} frames"))
            .unwrap_or_else(|| "Default".to_string())
    );

    // Open output with the selected configuration
    let output = CpalOutput::open_with_config(CpalOutputConfig {
        driver,
        device_id,
        sample_rate,
        buffer_size,
    })?;

    // Create player and play file
    let (status_sender, status_receiver) = sync_channel(32);
    let mut player = Player::new(output, status_sender);
    player.stop();

    let handle = player.play_file(
        &file_path,
        FilePlaybackOptions::default()
            .streamed()
            .playback_pos_emit_rate(Duration::from_secs(1)),
    )?;

    std::thread::spawn(move || {
        while let Ok(event) = status_receiver.recv() {
            match event {
                PlaybackStatusEvent::Position { path, position, .. } => {
                    println!("Playing '{}': {:.1}s", path, position.as_secs_f32());
                }
                PlaybackStatusEvent::Stopped {
                    path, exhausted, ..
                } => {
                    if exhausted {
                        println!("Finished '{path}'");
                    } else {
                        println!("Stopped '{path}'");
                    }
                }
            }
        }
    });

    // Resume playback
    player.start();

    // Wait until playback finished
    while handle.is_playing() {
        std::thread::sleep(Duration::from_millis(100));
    }

    Ok(())
}
