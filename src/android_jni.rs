#![cfg(target_os = "android")]

use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

use jni::objects::{JByteArray, JClass, JFloatArray, JString};
use jni::sys::{jboolean, jdouble, jfloat, jint, jlong, JNI_TRUE};
use jni::JNIEnv;

use crate::audio::android_engine::{SonoraMobileEngine, VISUALIZER_FLOAT_LEN};
use crate::config::AppConfig;

static ENGINE: OnceLock<Mutex<SonoraMobileEngine>> = OnceLock::new();

fn with_engine<R>(f: impl FnOnce(&mut SonoraMobileEngine) -> R) -> Option<R> {
    let lock = ENGINE.get()?;
    let mut guard = lock.lock().ok()?;
    Some(f(&mut guard))
}

fn jstr_to_string(env: &mut JNIEnv, s: JString) -> String {
    env.get_string(&s)
        .map(|js| js.into())
        .unwrap_or_default()
}

fn string_to_jstr<'local>(env: &mut JNIEnv<'local>, s: &str) -> JString<'local> {
    env.new_string(s)
        .unwrap_or_else(|_| env.new_string("").expect("empty jstring"))
}

#[no_mangle]
pub extern "system" fn Java_com_sonora_hiresplayer_SonoraNative_nativeInit(
    mut env: JNIEnv,
    _cls: JClass,
    files_dir: JString,
) {
    let _ = android_logger::init_once(
        android_logger::Config::default()
            .with_max_level(log::LevelFilter::Info)
            .with_tag("SonoraCore"),
    );
    let dir = jstr_to_string(&mut env, files_dir);
    if !dir.is_empty() {
        AppConfig::set_data_dir_override(PathBuf::from(dir));
    }
    let _ = ENGINE.get_or_init(|| Mutex::new(SonoraMobileEngine::new()));
}

#[no_mangle]
pub extern "system" fn Java_com_sonora_hiresplayer_SonoraNative_nativeGetStateJson<'local>(
    mut env: JNIEnv<'local>,
    _cls: JClass,
) -> JString<'local> {
    let json = with_engine(|eng| {
        let snap = eng.snapshot();
        serde_json::to_string(&snap).unwrap_or_else(|_| "{}".to_string())
    })
    .unwrap_or_else(|| "{}".to_string());
    string_to_jstr(&mut env, &json)
}

#[no_mangle]
pub extern "system" fn Java_com_sonora_hiresplayer_SonoraNative_nativeRenderAudio(
    env: JNIEnv,
    _cls: JClass,
    out_buf: JFloatArray,
) -> jint {
    let len = match env.get_array_length(&out_buf) {
        Ok(l) if l > 0 => l as usize,
        _ => return 0,
    };
    let mut scratch = vec![0.0_f32; len];
    let n = with_engine(|eng| eng.render_audio(&mut scratch)).unwrap_or(0);
    if n > 0 {
        let written = (n as usize).min(len);
        let _ = env.set_float_array_region(&out_buf, 0, &scratch[..written]);
    }
    n
}

#[no_mangle]
pub extern "system" fn Java_com_sonora_hiresplayer_SonoraNative_nativeGetDesiredOutputRate(
    _env: JNIEnv,
    _cls: JClass,
) -> jint {
    with_engine(|eng| eng.desired_output_rate() as jint).unwrap_or(48_000)
}

#[no_mangle]
pub extern "system" fn Java_com_sonora_hiresplayer_SonoraNative_nativeSetActualOutputFormat(
    _env: JNIEnv,
    _cls: JClass,
    sample_rate: jint,
    channels: jint,
) {
    with_engine(|eng| {
        eng.set_actual_output_format(sample_rate.max(8_000) as u32, channels.max(1) as u16);
    });
}

#[no_mangle]
pub extern "system" fn Java_com_sonora_hiresplayer_SonoraNative_nativeGetVisualizerFloats<'local>(
    env: JNIEnv<'local>,
    _cls: JClass,
) -> JFloatArray<'local> {
    let floats = with_engine(|eng| eng.sample_visualizer_floats())
        .unwrap_or([0.0_f32; VISUALIZER_FLOAT_LEN]);
    let arr = env
        .new_float_array(VISUALIZER_FLOAT_LEN as i32)
        .expect("new_float_array");
    let _ = env.set_float_array_region(&arr, 0, &floats);
    arr
}

#[no_mangle]
pub extern "system" fn Java_com_sonora_hiresplayer_SonoraNative_nativeGetCoverArtBytes<'local>(
    env: JNIEnv<'local>,
    _cls: JClass,
) -> JByteArray<'local> {
    let bytes = with_engine(|eng| eng.cover_art_bytes().map(|b| b.to_vec()))
        .flatten()
        .unwrap_or_default();
    env.byte_array_from_slice(&bytes)
        .expect("byte_array_from_slice")
}

#[no_mangle]
pub extern "system" fn Java_com_sonora_hiresplayer_SonoraNative_nativePlay(
    _env: JNIEnv,
    _cls: JClass,
) {
    with_engine(|eng| eng.play());
}

#[no_mangle]
pub extern "system" fn Java_com_sonora_hiresplayer_SonoraNative_nativePause(
    _env: JNIEnv,
    _cls: JClass,
) {
    with_engine(|eng| eng.pause());
}

#[no_mangle]
pub extern "system" fn Java_com_sonora_hiresplayer_SonoraNative_nativeTogglePlayPause(
    _env: JNIEnv,
    _cls: JClass,
) {
    with_engine(|eng| eng.toggle_play_pause());
}

#[no_mangle]
pub extern "system" fn Java_com_sonora_hiresplayer_SonoraNative_nativeStop(
    _env: JNIEnv,
    _cls: JClass,
) {
    with_engine(|eng| eng.stop());
}

#[no_mangle]
pub extern "system" fn Java_com_sonora_hiresplayer_SonoraNative_nativeSeek(
    _env: JNIEnv,
    _cls: JClass,
    seconds: jdouble,
) {
    with_engine(|eng| eng.seek(seconds));
}

#[no_mangle]
pub extern "system" fn Java_com_sonora_hiresplayer_SonoraNative_nativeNext(
    _env: JNIEnv,
    _cls: JClass,
    user_triggered: jboolean,
) {
    with_engine(|eng| eng.next_track(user_triggered == JNI_TRUE));
}

#[no_mangle]
pub extern "system" fn Java_com_sonora_hiresplayer_SonoraNative_nativePrev(
    _env: JNIEnv,
    _cls: JClass,
) {
    with_engine(|eng| eng.prev_track());
}

#[no_mangle]
pub extern "system" fn Java_com_sonora_hiresplayer_SonoraNative_nativeLoadTrack(
    mut env: JNIEnv,
    _cls: JClass,
    playlist_id: JString,
    index: jint,
    autoplay: jboolean,
) {
    let pid = jstr_to_string(&mut env, playlist_id);
    if index >= 0 {
        with_engine(|eng| eng.load_track(&pid, index as usize, autoplay == JNI_TRUE));
    }
}

#[no_mangle]
pub extern "system" fn Java_com_sonora_hiresplayer_SonoraNative_nativeSetVolume(
    _env: JNIEnv,
    _cls: JClass,
    vol: jfloat,
) {
    with_engine(|eng| eng.set_volume(vol));
}

#[no_mangle]
pub extern "system" fn Java_com_sonora_hiresplayer_SonoraNative_nativeSetMuted(
    _env: JNIEnv,
    _cls: JClass,
    muted: jboolean,
) {
    with_engine(|eng| eng.set_muted(muted == JNI_TRUE));
}

#[no_mangle]
pub extern "system" fn Java_com_sonora_hiresplayer_SonoraNative_nativeToggleShuffle(
    _env: JNIEnv,
    _cls: JClass,
) {
    with_engine(|eng| eng.toggle_shuffle());
}

#[no_mangle]
pub extern "system" fn Java_com_sonora_hiresplayer_SonoraNative_nativeCycleRepeat(
    _env: JNIEnv,
    _cls: JClass,
) {
    with_engine(|eng| eng.cycle_repeat());
}

#[no_mangle]
pub extern "system" fn Java_com_sonora_hiresplayer_SonoraNative_nativeUpdateConfigJson(
    mut env: JNIEnv,
    _cls: JClass,
    json: JString,
) {
    let raw = jstr_to_string(&mut env, json);
    if let Ok(cfg) = serde_json::from_str::<AppConfig>(&raw) {
        with_engine(|eng| eng.update_config(cfg));
    }
}

#[no_mangle]
pub extern "system" fn Java_com_sonora_hiresplayer_SonoraNative_nativeCycleAutoVolume(
    _env: JNIEnv,
    _cls: JClass,
) {
    with_engine(|eng| eng.cycle_auto_volume_mode());
}

#[no_mangle]
pub extern "system" fn Java_com_sonora_hiresplayer_SonoraNative_nativeReanalyzeAutoVolume(
    _env: JNIEnv,
    _cls: JClass,
) {
    with_engine(|eng| eng.reanalyze_auto_volume());
}

#[no_mangle]
pub extern "system" fn Java_com_sonora_hiresplayer_SonoraNative_nativeSetChannelMuteSolo(
    _env: JNIEnv,
    _cls: JClass,
    channel: jint,
    mute: jboolean,
    solo: jboolean,
) {
    if channel >= 0 {
        with_engine(|eng| {
            eng.set_channel_mute_solo(channel as usize, mute == JNI_TRUE, solo == JNI_TRUE)
        });
    }
}

#[no_mangle]
pub extern "system" fn Java_com_sonora_hiresplayer_SonoraNative_nativeResetMuteSolo(
    _env: JNIEnv,
    _cls: JClass,
) {
    with_engine(|eng| eng.reset_mute_solo());
}

#[no_mangle]
pub extern "system" fn Java_com_sonora_hiresplayer_SonoraNative_nativeImportPathsJson(
    mut env: JNIEnv,
    _cls: JClass,
    paths_json: JString,
) -> jint {
    let raw = jstr_to_string(&mut env, paths_json);
    let paths: Vec<PathBuf> = serde_json::from_str::<Vec<String>>(&raw)
        .unwrap_or_default()
        .into_iter()
        .map(PathBuf::from)
        .collect();
    with_engine(|eng| eng.import_paths(paths) as jint).unwrap_or(0)
}

#[no_mangle]
pub extern "system" fn Java_com_sonora_hiresplayer_SonoraNative_nativeCreatePlaylist(
    mut env: JNIEnv,
    _cls: JClass,
    name: JString,
    copy_current: jboolean,
    seed_index: jint,
) {
    let n = jstr_to_string(&mut env, name);
    let seed = if seed_index >= 0 {
        Some(seed_index as usize)
    } else {
        None
    };
    with_engine(|eng| eng.create_playlist(&n, copy_current == JNI_TRUE, seed));
}

#[no_mangle]
pub extern "system" fn Java_com_sonora_hiresplayer_SonoraNative_nativeSwitchPlaylist(
    mut env: JNIEnv,
    _cls: JClass,
    id: JString,
) {
    let pid = jstr_to_string(&mut env, id);
    with_engine(|eng| eng.switch_playlist(&pid));
}

#[no_mangle]
pub extern "system" fn Java_com_sonora_hiresplayer_SonoraNative_nativeRenamePlaylist(
    mut env: JNIEnv,
    _cls: JClass,
    id: JString,
    new_name: JString,
) {
    let pid = jstr_to_string(&mut env, id);
    let nn = jstr_to_string(&mut env, new_name);
    with_engine(|eng| eng.rename_playlist(&pid, &nn));
}

#[no_mangle]
pub extern "system" fn Java_com_sonora_hiresplayer_SonoraNative_nativeDuplicatePlaylist(
    mut env: JNIEnv,
    _cls: JClass,
    id: JString,
) {
    let pid = jstr_to_string(&mut env, id);
    with_engine(|eng| eng.duplicate_playlist(&pid));
}

#[no_mangle]
pub extern "system" fn Java_com_sonora_hiresplayer_SonoraNative_nativeDeletePlaylist(
    mut env: JNIEnv,
    _cls: JClass,
    id: JString,
) {
    let pid = jstr_to_string(&mut env, id);
    with_engine(|eng| eng.delete_playlist(&pid));
}

#[no_mangle]
pub extern "system" fn Java_com_sonora_hiresplayer_SonoraNative_nativeAddTrackToPlaylist(
    mut env: JNIEnv,
    _cls: JClass,
    target_id: JString,
    source_index: jint,
) {
    let tid = jstr_to_string(&mut env, target_id);
    if source_index >= 0 {
        with_engine(|eng| eng.add_track_to_playlist(&tid, source_index as usize));
    }
}

#[no_mangle]
pub extern "system" fn Java_com_sonora_hiresplayer_SonoraNative_nativeRemoveTrack(
    _env: JNIEnv,
    _cls: JClass,
    index: jint,
) {
    if index >= 0 {
        with_engine(|eng| eng.remove_track(index as usize));
    }
}

#[no_mangle]
pub extern "system" fn Java_com_sonora_hiresplayer_SonoraNative_nativeMoveTrack(
    _env: JNIEnv,
    _cls: JClass,
    from: jint,
    to: jint,
) {
    if from >= 0 && to >= 0 {
        with_engine(|eng| eng.move_track(from as usize, to as usize));
    }
}

#[no_mangle]
pub extern "system" fn Java_com_sonora_hiresplayer_SonoraNative_nativeClearActivePlaylist(
    _env: JNIEnv,
    _cls: JClass,
) {
    with_engine(|eng| eng.clear_active_playlist());
}

#[no_mangle]
pub extern "system" fn Java_com_sonora_hiresplayer_SonoraNative_nativeRestoreStudioReferences(
    _env: JNIEnv,
    _cls: JClass,
) {
    with_engine(|eng| eng.restore_studio_references());
}

#[no_mangle]
pub extern "system" fn Java_com_sonora_hiresplayer_SonoraNative_nativeUploadLyricsText(
    mut env: JNIEnv,
    _cls: JClass,
    raw_text: JString,
    source_label: JString,
) {
    let raw = jstr_to_string(&mut env, raw_text);
    let src = jstr_to_string(&mut env, source_label);
    with_engine(|eng| eng.upload_lyrics_text(&raw, &src));
}

#[no_mangle]
pub extern "system" fn Java_com_sonora_hiresplayer_SonoraNative_nativeSetLyricsOffsetMs(
    _env: JNIEnv,
    _cls: JClass,
    offset_ms: jlong,
) {
    with_engine(|eng| eng.set_lyrics_offset_ms(offset_ms));
}

#[no_mangle]
pub extern "system" fn Java_com_sonora_hiresplayer_SonoraNative_nativeGetLyricsRaw<'local>(
    mut env: JNIEnv<'local>,
    _cls: JClass,
) -> JString<'local> {
    let raw = with_engine(|eng| eng.get_lyrics_raw()).unwrap_or_default();
    string_to_jstr(&mut env, &raw)
}
