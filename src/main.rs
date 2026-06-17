use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use clap::Parser;
use serde::Deserialize;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Parser, Debug)]
struct Args {
    #[arg(short, long)]
    input: PathBuf,

    #[arg(short, long, default_value = "final")]
    output: PathBuf,
}

#[derive(Debug, Deserialize)]
struct GooglePhotoMetadata {
    title: String,
    #[serde(default)]
    description: String,
    #[serde(default, rename = "imageViews")]
    image_views: Option<String>,
    #[serde(rename = "creationTime")]
    creation_time: GoogleTime,
    #[serde(rename = "photoTakenTime")]
    photo_taken_time: GoogleTime,
    #[serde(default, rename = "geoData")]
    geo_data: GeoData,
    #[serde(default)]
    url: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GoogleTime {
    timestamp: String,
    #[serde(default, rename = "formatted")]
    _formatted: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct GeoData {
    #[serde(default)]
    latitude: f64,
    #[serde(default)]
    longitude: f64,
    #[serde(default)]
    altitude: f64,
    #[serde(default, rename = "latitudeSpan")]
    _latitude_span: Option<f64>,
    #[serde(default, rename = "longitudeSpan")]
    _longitude_span: Option<f64>,
}

#[derive(Debug, Deserialize)]
struct MetadataTitle {
    title: String,
}

fn main() -> Result<()> {
    let args = Args::parse();

    println!("chemin de input : {}", args.input.display());
    println!("chemin de output : {}", args.output.display());

    fs::create_dir_all(&args.output).with_context(|| {
        format!(
            "impossible de creer le dossier de sortie {}",
            args.output.display()
        )
    })?;

    let (media_files, json_files) = list_files(&args.input)?;
    let mut processed = 0;
    let mut skipped = 0;

    for media_path in media_files {
        let media_name = file_name_string(&media_path)?;

        let Some(json_path) = find_json_for_media(&media_path, &json_files)? else {
            eprintln!("aucun JSON trouve pour {}", media_name);
            skipped += 1;
            continue;
        };

        let output_media_path = args.output.join(&media_name);
        fs::copy(&media_path, &output_media_path).with_context(|| {
            format!(
                "impossible de copier {} vers {}",
                media_path.display(),
                output_media_path.display()
            )
        })?;

        println!(
            "l'image {} va avec le json {}",
            media_name,
            json_path.display()
        );

        write_metadata(&output_media_path, &json_path).with_context(|| {
            format!(
                "impossible d'ecrire les metadata de {}",
                output_media_path.display()
            )
        })?;

        processed += 1;
    }

    println!("{} fichier(s) traite(s), {} ignore(s)", processed, skipped);

    Ok(())
}

fn list_files(input: &Path) -> Result<(Vec<PathBuf>, Vec<PathBuf>)> {
    let mut media_files = Vec::new();
    let mut json_files = Vec::new();

    for entry in fs::read_dir(input)
        .with_context(|| format!("impossible de lire le dossier {}", input.display()))?
    {
        let entry = entry?;
        let path = entry.path();

        if !entry.file_type()?.is_file() {
            continue;
        }

        if has_extension(&path, "json") {
            json_files.push(path);
        } else if is_supported_media(&path) {
            media_files.push(path);
        }
    }

    Ok((media_files, json_files))
}

fn find_json_for_media(media_path: &Path, json_files: &[PathBuf]) -> Result<Option<PathBuf>> {
    let media_name = file_name_string(media_path)?;
    let media_stem = media_path
        .file_stem()
        .and_then(|name| name.to_str())
        .context("nom de media invalide")?;

    for json_path in json_files {
        let json_name = file_name_string(json_path)?;

        if json_name.eq_ignore_ascii_case(&format!("{media_name}.json"))
            || json_name.eq_ignore_ascii_case(&format!("{media_stem}.json"))
            || starts_with_ignore_case(&json_name, &format!("{media_name}.supplemental-"))
            || starts_with_ignore_case(&json_name, &format!("{media_name}.suppl"))
            || starts_with_ignore_case(&json_name, &format!("{media_stem}.supplemental-"))
            || starts_with_ignore_case(&json_name, &format!("{media_stem}.suppl"))
        {
            return Ok(Some(json_path.clone()));
        }
    }

    for json_path in json_files {
        if metadata_title(json_path).as_deref() == Some(media_name.as_str()) {
            return Ok(Some(json_path.clone()));
        }
    }

    Ok(None)
}

fn metadata_title(json_path: &Path) -> Option<String> {
    let json_content = fs::read_to_string(json_path).ok()?;
    let metadata: MetadataTitle = serde_json::from_str(&json_content).ok()?;
    Some(metadata.title)
}

fn unix_to_exiftool_date(timestamp: &str) -> Result<String> {
    let ts: i64 = timestamp.parse().context("timestamp invalide")?;

    let dt: DateTime<Utc> =
        DateTime::from_timestamp(ts, 0).context("impossible de convertir le timestamp")?;

    Ok(dt.format("%Y:%m:%d %H:%M:%S").to_string())
}

fn write_metadata(media_path: &Path, json_path: &Path) -> Result<()> {
    let json_content = fs::read_to_string(json_path)
        .with_context(|| format!("impossible de lire le fichier JSON {}", json_path.display()))?;

    let metadata: GooglePhotoMetadata =
        serde_json::from_str(&json_content).context("JSON Google Photos invalide")?;

    let creation_date = unix_to_exiftool_date(&metadata.creation_time.timestamp)?;
    let taken_date = unix_to_exiftool_date(&metadata.photo_taken_time.timestamp)?;

    let mut args: Vec<String> = Vec::new();

    args.push("-overwrite_original".to_string());
    args.push(format!("-Title={}", metadata.title));
    args.push(format!("-Description={}", metadata.description));

    if let Some(url) = &metadata.url {
        args.push(format!("-XMP:Identifier={}", url));
        args.push(format!("-Source={}", url));
    }

    if let Some(views) = &metadata.image_views {
        args.push(format!("-Comment=Google Photos views: {}", views));
    }

    args.push(format!("-UserComment={}", json_content));

    if is_jpeg(media_path) {
        args.push(format!("-EXIF:ImageDescription={}", metadata.description));
        args.push(format!("-EXIF:DateTimeOriginal={}", taken_date));
        args.push(format!("-EXIF:CreateDate={}", creation_date));
        args.push(format!("-XMP:DateCreated={}", taken_date));
        args.push(format!("-XMP:CreateDate={}", creation_date));

        add_gps_args(&mut args, &metadata.geo_data, "GPS");
    } else if is_png(media_path) {
        args.push(format!("-PNG:Description={}", metadata.description));
        args.push(format!("-PNG:CreationTime={}", taken_date));
        args.push(format!("-XMP:DateCreated={}", taken_date));
        args.push(format!("-XMP:CreateDate={}", creation_date));

        add_gps_args(&mut args, &metadata.geo_data, "GPS");
    } else if is_mp4(media_path) {
        args.push(format!("-QuickTime:Title={}", metadata.title));
        args.push(format!("-QuickTime:Description={}", metadata.description));
        args.push(format!("-QuickTime:CreateDate={}", taken_date));
        args.push(format!("-QuickTime:ModifyDate={}", creation_date));

        if has_gps(&metadata.geo_data) {
            args.push(format!(
                "-QuickTime:GPSCoordinates={} {}",
                metadata.geo_data.latitude, metadata.geo_data.longitude
            ));
        }
    } else {
        anyhow::bail!("format non supporte : {:?}", media_path);
    }

    args.push(media_path.to_string_lossy().to_string());

    let output = Command::new("exiftool")
        .args(&args)
        .output()
        .context("impossible d'executer exiftool")?;

    if !output.status.success() {
        anyhow::bail!(
            "ExifTool a echoue:\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    print!("{}", String::from_utf8_lossy(&output.stdout));

    Ok(())
}

fn add_gps_args(args: &mut Vec<String>, geo_data: &GeoData, prefix: &str) {
    if !has_gps(geo_data) {
        return;
    }

    args.push(format!("-{prefix}:GPSLatitude={}", geo_data.latitude));
    args.push(format!("-{prefix}:GPSLongitude={}", geo_data.longitude));
    args.push(format!("-{prefix}:GPSAltitude={}", geo_data.altitude));
}

fn has_gps(geo_data: &GeoData) -> bool {
    geo_data.latitude != 0.0 || geo_data.longitude != 0.0
}

fn is_supported_media(path: &Path) -> bool {
    is_jpeg(path) || is_png(path) || is_mp4(path)
}

fn is_mp4(path: &Path) -> bool {
    has_extension(path, "mp4")
}

fn is_png(path: &Path) -> bool {
    has_extension(path, "png")
}

fn is_jpeg(path: &Path) -> bool {
    has_extension(path, "jpg") || has_extension(path, "jpeg")
}

fn has_extension(path: &Path, extension: &str) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.eq_ignore_ascii_case(extension))
        .unwrap_or(false)
}

fn starts_with_ignore_case(text: &str, prefix: &str) -> bool {
    text.get(..prefix.len())
        .map(|start| start.eq_ignore_ascii_case(prefix))
        .unwrap_or(false)
}

fn file_name_string(path: &Path) -> Result<String> {
    path.file_name()
        .and_then(|name| name.to_str())
        .map(ToOwned::to_owned)
        .context("nom de fichier invalide")
}
