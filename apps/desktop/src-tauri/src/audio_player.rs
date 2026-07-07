use std::{
    fs::File,
    io::BufReader,
    path::{Path, PathBuf},
    sync::mpsc::{Receiver, Sender},
};

use rodio::{Decoder, OutputStream, OutputStreamHandle, Sink};
use yuukei_protocol::RuntimeCommand;

// rodioのOutputStreamは!Sendなので、再生専用スレッドに閉じ込めて
// AppState側にはSendなチャネルだけを持たせる。
pub struct AudioPlayer {
    sender: Sender<PathBuf>,
}

impl AudioPlayer {
    pub fn new() -> Result<Self, String> {
        let (sender, receiver) = std::sync::mpsc::channel::<PathBuf>();
        std::thread::Builder::new()
            .name("yuukei-audio-player".to_string())
            .spawn(move || playback_loop(receiver))
            .map_err(|error| format!("failed to spawn audio player thread: {error}"))?;
        Ok(Self { sender })
    }

    pub fn play_command(&self, command: &RuntimeCommand) -> Result<(), String> {
        let Some(path) = audio_path_from_command(command)? else {
            return Ok(());
        };
        self.sender
            .send(path)
            .map_err(|_| "audio player thread is gone".to_string())
    }
}

fn playback_loop(receiver: Receiver<PathBuf>) {
    let mut output: Option<(OutputStream, OutputStreamHandle)> = None;
    let mut current: Option<Sink> = None;
    while let Ok(path) = receiver.recv() {
        if output.is_none() {
            match OutputStream::try_default() {
                Ok(pair) => output = Some(pair),
                Err(error) => {
                    eprintln!("Yuukei audio output unavailable: {error}");
                    continue;
                }
            }
        }
        let Some((_, handle)) = output.as_ref() else {
            continue;
        };
        match decode_into_sink(&path, handle) {
            Ok(sink) => {
                if let Some(previous) = current.replace(sink) {
                    previous.stop();
                }
            }
            Err(error) => eprintln!("Yuukei audio playback failed: {error}"),
        }
    }
}

fn decode_into_sink(path: &Path, handle: &OutputStreamHandle) -> Result<Sink, String> {
    let file = File::open(path)
        .map_err(|error| format!("failed to open audio file {}: {error}", path.display()))?;
    let source = Decoder::new(BufReader::new(file))
        .map_err(|error| format!("failed to decode wav {}: {error}", path.display()))?;
    let sink = Sink::try_new(handle).map_err(|error| error.to_string())?;
    sink.append(source);
    sink.play();
    Ok(sink)
}

pub fn audio_path_from_command(command: &RuntimeCommand) -> Result<Option<PathBuf>, String> {
    if command.kind != "audio.play" {
        return Ok(None);
    }
    let Some(path) = command
        .payload
        .get("audioPath")
        .and_then(|value| value.as_str())
    else {
        return Err("audio.play missing audioPath".to_string());
    };
    let path = PathBuf::from(path);
    validate_wav_path(&path)?;
    Ok(Some(path))
}

fn validate_wav_path(path: &Path) -> Result<(), String> {
    if path
        .extension()
        .and_then(|extension| extension.to_str())
        .is_none_or(|extension| !extension.eq_ignore_ascii_case("wav"))
    {
        return Err(format!(
            "audio.play path is not a wav file: {}",
            path.display()
        ));
    }
    let metadata = path
        .metadata()
        .map_err(|error| format!("audio.play path unavailable {}: {error}", path.display()))?;
    if !metadata.is_file() {
        return Err(format!("audio.play path is not a file: {}", path.display()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fs;

    use serde_json::json;
    use tempfile::tempdir;

    use super::*;

    #[test]
    fn audio_path_from_command_accepts_existing_wav_file() -> Result<(), Box<dyn std::error::Error>>
    {
        let dir = tempdir()?;
        let path = dir.path().join("voice.wav");
        fs::write(&path, b"RIFF")?;
        let mut command = RuntimeCommand::new("audio.play", "capability", "resident-default");
        command
            .payload
            .insert("audioPath".to_string(), json!(path.to_string_lossy()));

        assert_eq!(audio_path_from_command(&command)?, Some(path));
        Ok(())
    }

    #[test]
    fn audio_path_from_command_ignores_non_audio_commands() -> Result<(), Box<dyn std::error::Error>>
    {
        let command = RuntimeCommand::new("dialogue.say", "daihon", "resident-default");
        assert_eq!(audio_path_from_command(&command)?, None);
        Ok(())
    }

    #[test]
    fn audio_path_from_command_rejects_missing_and_non_wav_paths(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempdir()?;
        let path = dir.path().join("voice.mp3");
        fs::write(&path, b"not wav")?;
        let mut command = RuntimeCommand::new("audio.play", "capability", "resident-default");
        command
            .payload
            .insert("audioPath".to_string(), json!(path.to_string_lossy()));
        assert!(audio_path_from_command(&command)
            .unwrap_err()
            .contains("not a wav"));

        command.payload.insert(
            "audioPath".to_string(),
            json!(dir.path().join("missing.wav").to_string_lossy()),
        );
        assert!(audio_path_from_command(&command)
            .unwrap_err()
            .contains("unavailable"));
        Ok(())
    }
}
