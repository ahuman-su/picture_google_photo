use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use clap::Parser;
use indicatif::{ProgressBar, ProgressStyle};
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
    #[serde(default, rename = "geoDataExif")]
    geo_data_exif: GeoData,
    #[serde(default)]
    url: Option<String>,
}

impl GooglePhotoMetadata {
    fn gps_data(&self) -> Option<&GeoData> {
        if self.geo_data_exif.has_coordinates() {
            Some(&self.geo_data_exif)
        } else if self.geo_data.has_coordinates() {
            Some(&self.geo_data)
        } else {
            None
        }
    }
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

impl GeoData {
    fn has_coordinates(&self) -> bool {
        self.latitude != 0.0 || self.longitude != 0.0
    }
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
    let progress = ProgressBar::new(media_files.len() as u64);
    progress.set_style(
        ProgressStyle::with_template(
            "[{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len} {percent}% {wide_msg}",
        )
        .context("template de barre d'avancement invalide")?
        .progress_chars("=>-"),
    );

    let mut copied = 0;
    let mut metadata_written = 0;
    let mut metadata_errors = 0;
    let mut copy_errors = 0;
    let mut skipped = 0;

    for media_path in media_files {
        let media_name = file_name_string(&media_path)?;
        progress.set_message(media_name.clone());

        let Some(json_path) = find_json_for_media(&media_path, &json_files)? else {
            skipped += 1;
            progress.inc(1);
            continue;
        };

        let output_media_path = args.output.join(&media_name);
        if fs::copy(&media_path, &output_media_path).is_err() {
            copy_errors += 1;
            progress.inc(1);
            continue;
        }

        copied += 1;

        match write_metadata(&output_media_path, &json_path) {
            Ok(()) => metadata_written += 1,
            Err(_) => {
                metadata_errors += 1;

                if fs::copy(&media_path, &output_media_path).is_err() {
                    copy_errors += 1;
                }
            }
        }

        progress.inc(1);
    }

    progress.finish_and_clear();

    let total_errors = metadata_errors + copy_errors;
    println!("{copied} fichier(s) copie(s)");
    println!("{metadata_written} fichier(s) avec metadata");
    println!("{skipped} fichier(s) ignore(s) sans JSON");
    println!("{total_errors} erreur(s)");
    println!("{metadata_errors} erreur(s) metadata");
    println!("{copy_errors} erreur(s) copie");

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

        if let Some(geo_data) = metadata.gps_data() {
            add_image_gps_args(&mut args, geo_data);
        }
    } else if is_png(media_path) {
        args.push(format!("-PNG:Description={}", metadata.description));
        args.push(format!("-PNG:CreationTime={}", taken_date));
        args.push(format!("-XMP:DateCreated={}", taken_date));
        args.push(format!("-XMP:CreateDate={}", creation_date));

        if let Some(geo_data) = metadata.gps_data() {
            add_image_gps_args(&mut args, geo_data);
        }
    } else if is_mp4(media_path) {
        args.push(format!("-QuickTime:Title={}", metadata.title));
        args.push(format!("-QuickTime:Description={}", metadata.description));
        args.push(format!("-QuickTime:CreateDate={}", taken_date));
        args.push(format!("-QuickTime:ModifyDate={}", creation_date));

        if let Some(geo_data) = metadata.gps_data() {
            add_mp4_gps_args(&mut args, geo_data);
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

    Ok(())
}

fn add_image_gps_args(args: &mut Vec<String>, geo_data: &GeoData) {
    let latitude_ref = if geo_data.latitude < 0.0 { "S" } else { "N" };
    let longitude_ref = if geo_data.longitude < 0.0 { "W" } else { "E" };
    let altitude_ref = if geo_data.altitude < 0.0 {
        "Below Sea Level"
    } else {
        "Above Sea Level"
    };

    args.push(format!("-EXIF:GPSLatitude={}", geo_data.latitude.abs()));
    args.push(format!("-EXIF:GPSLatitudeRef={latitude_ref}"));
    args.push(format!("-EXIF:GPSLongitude={}", geo_data.longitude.abs()));
    args.push(format!("-EXIF:GPSLongitudeRef={longitude_ref}"));
    args.push(format!("-EXIF:GPSAltitude={}", geo_data.altitude.abs()));
    args.push(format!("-EXIF:GPSAltitudeRef={altitude_ref}"));
}

fn add_mp4_gps_args(args: &mut Vec<String>, geo_data: &GeoData) {
    let coordinates = iso_6709_coordinates(geo_data);

    args.push(format!("-Keys:GPSCoordinates={coordinates}"));
    args.push(format!("-UserData:GPSCoordinates={coordinates}"));
}

fn iso_6709_coordinates(geo_data: &GeoData) -> String {
    format!(
        "{:+010.6}{:+011.6}{:+.3}/",
        geo_data.latitude, geo_data.longitude, geo_data.altitude
    )
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
