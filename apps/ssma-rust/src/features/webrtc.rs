use crate::gateway::{apply_audio_backend_event, emit_rtc_signal, AppState, AudioSessionRecord};
use anyhow::anyhow;
use base64::Engine;
use bytes::Bytes;
use futures_util::{SinkExt, StreamExt};
use hound::WavReader;
use rustrtc::media::track::{sample_track, SampleStreamSource};
use rustrtc::media::{AudioFrame, MediaKind as RtcMediaKind, MediaSample, MediaStreamTrack};
use rustrtc::peer_connection::{PeerConnection, PeerConnectionEvent};
use rustrtc::transports::ice::IceCandidate;
use rustrtc::{AudioCapability, RtcConfiguration, RtpCodecParameters, SdpType, SessionDescription};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::net::TcpStream;
use tokio::sync::{mpsc, RwLock};
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::Message as WsMessage;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};
use tracing::{error, warn};
use voxudio::{OpusApplication, OpusCodec, OpusDecoder as VoxOpusDecoder, OpusEncoder as VoxOpusEncoder};

const RTC_AUDIO_SAMPLE_RATE: u32 = 48_000;
const RTC_AUDIO_CHANNELS: usize = 2;
const OPUS_FRAME_MS: usize = 20;
const OPUS_FRAME_SAMPLES_PER_CHANNEL: usize =
    (RTC_AUDIO_SAMPLE_RATE as usize * OPUS_FRAME_MS) / 1000;

type UpstreamReader = futures_util::stream::SplitStream<WebSocketStream<MaybeTlsStream<TcpStream>>>;

#[derive(Debug, Clone)]
pub struct IncomingRtcSignal {
    pub kind: String,
    pub sender_id: String,
    pub target_id: Option<String>,
    pub payload: Value,
}

#[derive(Clone, Default)]
pub struct WebRtcManager {
    bridges: Arc<RwLock<HashMap<String, WebRtcBridgeHandle>>>,
}

#[derive(Clone)]
struct WebRtcBridgeHandle {
    control_tx: mpsc::Sender<BridgeCommand>,
}

#[derive(Debug)]
enum BridgeCommand {
    BrowserSignal(IncomingRtcSignal),
    AudioCommand { command: String, payload: Value },
    Close,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ModelsLocalWsMessage {
    #[allow(dead_code)]
    SessionSnapshot { session: ModelsLocalAudioSession },
    SessionEvent { event: ModelsLocalAudioEvent },
    Error { error: String },
    Pong,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ModelsLocalAudioSessionEnvelope {
    session: ModelsLocalAudioSession,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ModelsLocalAudioSession {
    session_id: String,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct ModelsLocalAudioEvent {
    session_id: String,
    event_type: String,
    status: String,
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    mime_type: Option<String>,
    #[serde(default)]
    encoding: Option<String>,
    #[serde(default)]
    sample_rate: Option<u32>,
    #[serde(default)]
    audio_chunk_base64: Option<String>,
    #[serde(default)]
    sequence: Option<u64>,
    final_segment: bool,
    created_at: u64,
}

impl WebRtcManager {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) async fn ensure_bridge(
        &self,
        state: Arc<AppState>,
        session: &AudioSessionRecord,
    ) -> anyhow::Result<()> {
        let mut bridges = self.bridges.write().await;
        if bridges.contains_key(&session.session_id) {
            return Ok(());
        }
        let (control_tx, control_rx) = mpsc::channel(128);
        let session_id = session.session_id.clone();
        let site = session.site.clone();
        let rtc_session_id = session.rtc_session_id.clone();
        let backend = session.backend.clone();
        tokio::spawn(async move {
            if let Err(error) =
                run_bridge(state.clone(), session_id.clone(), site, rtc_session_id, backend, control_rx).await
            {
                error!(audio_session_id = %session_id, ?error, "webrtc bridge failed");
                let _ = apply_audio_backend_event(
                    &state,
                    "default",
                    &session_id,
                    "audio.session.error",
                    json!({ "message": error.to_string() }),
                );
            }
        });
        bridges.insert(session.session_id.clone(), WebRtcBridgeHandle { control_tx });
        Ok(())
    }

    pub(crate) async fn handle_command(
        &self,
        session_id: &str,
        command: &str,
        payload: Value,
    ) -> anyhow::Result<()> {
        let bridges = self.bridges.read().await;
        let handle = bridges
            .get(session_id)
            .ok_or_else(|| anyhow!("missing webrtc bridge for {session_id}"))?;
        handle
            .control_tx
            .send(BridgeCommand::AudioCommand {
                command: command.to_string(),
                payload,
            })
            .await
            .map_err(|_| anyhow!("webrtc bridge channel closed"))?;
        Ok(())
    }

    pub(crate) async fn handle_browser_signal(
        &self,
        session_id: &str,
        signal: IncomingRtcSignal,
    ) -> anyhow::Result<()> {
        let bridges = self.bridges.read().await;
        let handle = bridges
            .get(session_id)
            .ok_or_else(|| anyhow!("missing webrtc bridge for {session_id}"))?;
        handle
            .control_tx
            .send(BridgeCommand::BrowserSignal(signal))
            .await
            .map_err(|_| anyhow!("webrtc bridge channel closed"))?;
        Ok(())
    }

    pub(crate) async fn close_session(&self, session_id: &str) {
        let handle = self.bridges.write().await.remove(session_id);
        if let Some(handle) = handle {
            let _ = handle.control_tx.send(BridgeCommand::Close).await;
        }
    }
}

async fn run_bridge(
    state: Arc<AppState>,
    audio_session_id: String,
    site: String,
    rtc_session_id: String,
    _backend: String,
    mut control_rx: mpsc::Receiver<BridgeCommand>,
) -> anyhow::Result<()> {
    tracing::info!(
        audio_session_id = %audio_session_id,
        rtc_session_id = %rtc_session_id,
        "starting ssma webrtc bridge"
    );
    let mut rtc_config = RtcConfiguration::default();
    let mut capabilities = rustrtc::config::MediaCapabilities::default();
    capabilities.audio = vec![AudioCapability::opus()];
    capabilities.video.clear();
    capabilities.application = None;
    rtc_config.media_capabilities = Some(capabilities);

    let pc = PeerConnection::new(rtc_config);
    let remote_sender_id = Arc::new(RwLock::new(None::<String>));
    let (audio_source, outgoing_track, _feedback_rx) = sample_track(RtcMediaKind::Audio, 256);
    pc.add_track(
        outgoing_track,
        RtpCodecParameters {
            payload_type: 111,
            clock_rate: RTC_AUDIO_SAMPLE_RATE,
            channels: RTC_AUDIO_CHANNELS as u8,
        },
    )
    .map_err(|error| anyhow!("add outgoing audio track failed: {error}"))?;

    let state_for_candidates = state.clone();
    let site_for_candidates = site.clone();
    let rtc_session_for_candidates = rtc_session_id.clone();
    let remote_sender_for_candidates = remote_sender_id.clone();
    let mut ice_rx = pc.subscribe_ice_candidates();
    tokio::spawn(async move {
        while let Ok(candidate) = ice_rx.recv().await {
            let target_id = remote_sender_for_candidates.read().await.clone();
            let candidate_sdp = format!("candidate:{}", candidate.to_sdp());
            if let Err(error) = emit_rtc_signal(
                &state_for_candidates,
                &site_for_candidates,
                &rtc_session_for_candidates,
                "candidate",
                "ssma-webrtc",
                target_id,
                json!({
                    "candidate": candidate_sdp,
                    "sdpMid": "0",
                    "sdpMLineIndex": 0,
                }),
            ) {
                warn!("emit local ice candidate failed: {error:?}");
            }
        }
    });

    let (incoming_track_tx, mut incoming_track_rx) = mpsc::channel(1);
    let pc_events = pc.clone();
    tokio::spawn(async move {
        while let Some(event) = pc_events.recv().await {
            if let PeerConnectionEvent::Track(transceiver) = event {
                if transceiver.kind() == rustrtc::MediaKind::Audio {
                    if let Some(receiver) = transceiver.receiver() {
                        let _ = incoming_track_tx.send(receiver.track()).await;
                    }
                }
            }
        }
    });

    let mut pending_remote_candidates = Vec::<IceCandidate>::new();
    let mut incoming_track: Option<Arc<dyn MediaStreamTrack>> = None;
    let mut browser_audio_forward_started = false;
    let mut models_writer: Option<mpsc::Sender<WsMessage>> = None;
    let mut models_reader: Option<UpstreamReader> = None;
    let mut models_session_id: Option<String> = None;
    let mut stop_requested = false;
    let mut output_bytes = Vec::new();
    let mut output_mime = String::new();
    let mut output_encoding = String::new();
    let mut output_sample_rate = RTC_AUDIO_SAMPLE_RATE;
    let mut next_rtp_timestamp = 0u32;
    let mut logged_first_backend_event = false;
    let mut outbound_encoder = OpusEncoderBridge::new()?;

    loop {
        tokio::select! {
            maybe_track = incoming_track_rx.recv(), if incoming_track.is_none() => {
                if let Some(track) = maybe_track {
                    incoming_track = Some(track);
                    try_start_browser_audio_forward(&incoming_track, &models_writer, &mut browser_audio_forward_started)?;
                }
            }
            maybe_message = async {
                if let Some(reader) = &mut models_reader {
                    reader.next().await
                } else {
                    None
                }
            }, if models_reader.is_some() => {
                match maybe_message {
                    Some(Ok(WsMessage::Text(text))) => {
                        let should_close = handle_models_local_message(
                            &state,
                            &site,
                            &audio_session_id,
                            &rtc_session_id,
                            &audio_source,
                            &mut output_bytes,
                            &mut output_mime,
                            &mut output_encoding,
                            &mut output_sample_rate,
                            &mut next_rtp_timestamp,
                            &mut outbound_encoder,
                            stop_requested,
                            &mut logged_first_backend_event,
                            &text,
                        ).await?;
                        if should_close {
                            break;
                        }
                    }
                    Some(Ok(WsMessage::Binary(_))) => {}
                    Some(Ok(WsMessage::Close(_))) | None => break,
                    Some(Ok(_)) => {}
                    Some(Err(error)) => return Err(anyhow!("models_local realtime ws failed: {error}")),
                }
            }
            maybe_command = control_rx.recv() => {
                match maybe_command {
                    Some(BridgeCommand::BrowserSignal(signal)) => {
                        handle_browser_signal(
                            &state,
                            &site,
                            &rtc_session_id,
                            &pc,
                            &remote_sender_id,
                            &mut pending_remote_candidates,
                            signal,
                        ).await?;
                    }
                    Some(BridgeCommand::AudioCommand { command, payload }) => {
                        if command == "start" {
                            if models_session_id.is_none() {
                                let model = payload.get("model").and_then(Value::as_str).unwrap_or("liquid-audio");
                                let mode = payload.get("mode").and_then(Value::as_str).unwrap_or("speech_to_speech");
                                let prompt = payload.get("prompt").and_then(Value::as_str);
                                let session_value = match state.backend.create_audio_session(json!({
                                    "model": model,
                                    "mode": mode,
                                    "prompt": prompt,
                                })).await {
                                    Ok(value) => value,
                                    Err(error) => {
                                        warn!(audio_session_id = %audio_session_id, ?error, "create models_local live audio session failed");
                                        continue;
                                    }
                                };
                                let session: ModelsLocalAudioSessionEnvelope = match serde_json::from_value(session_value) {
                                    Ok(value) => value,
                                    Err(error) => {
                                        warn!(audio_session_id = %audio_session_id, ?error, "invalid models_local audio session response");
                                        continue;
                                    }
                                };
                                tracing::info!(
                                    audio_session_id = %audio_session_id,
                                    models_local_session_id = %session.session.session_id,
                                    "created models_local live audio session"
                                );
                                models_session_id = Some(session.session.session_id.clone());
                                let (writer, reader) = match connect_models_local_audio_ws(&state, &session.session.session_id).await {
                                    Ok(value) => value,
                                    Err(error) => {
                                        warn!(audio_session_id = %audio_session_id, ?error, "connect models_local live audio websocket failed");
                                        models_session_id = None;
                                        continue;
                                    }
                                };
                                models_writer = Some(writer);
                                models_reader = Some(reader);
                                try_start_browser_audio_forward(&incoming_track, &models_writer, &mut browser_audio_forward_started)?;
                            }
                        }
                        if let Some(session_id) = &models_session_id {
                            let response = state.backend.command_audio_session(
                                session_id,
                                json!({
                                    "command": command,
                                    "payload": payload,
                                }),
                            ).await?;
                            let _ = response;
                        }
                        if command == "stop" {
                            stop_requested = true;
                        } else if command == "interrupt" {
                            output_bytes.clear();
                            output_mime.clear();
                            output_encoding.clear();
                            output_sample_rate = RTC_AUDIO_SAMPLE_RATE;
                        }
                    }
                    Some(BridgeCommand::Close) | None => break,
                }
            }
        }
    }

    if let Some(session_id) = models_session_id {
        let _ = state.backend.delete_audio_session(&session_id).await;
    }
    pc.close();
    Ok(())
}

fn try_start_browser_audio_forward(
    incoming_track: &Option<Arc<dyn MediaStreamTrack>>,
    models_writer: &Option<mpsc::Sender<WsMessage>>,
    started: &mut bool,
) -> anyhow::Result<()> {
    if *started {
        return Ok(());
    }
    let Some(track) = incoming_track.clone() else {
        return Ok(());
    };
    let Some(models_tx) = models_writer.clone() else {
        return Ok(());
    };
    *started = true;
    tokio::spawn(async move {
        if let Err(error) = forward_browser_audio_track(track, models_tx).await {
            warn!("browser audio forwarding stopped: {error:?}");
        }
    });
    Ok(())
}

async fn connect_models_local_audio_ws(
    state: &Arc<AppState>,
    session_id: &str,
) -> anyhow::Result<(mpsc::Sender<WsMessage>, UpstreamReader)> {
    let ws_url = state
        .backend
        .ws_url(&format!("/internal/audio/sessions/{session_id}/ws"));
    let request = ws_url
        .into_client_request()
        .map_err(|error| anyhow!("build models_local ws request failed: {error}"))?;
    let (socket, _) = connect_async(request)
        .await
        .map_err(|error| anyhow!("models_local ws connect failed: {error}"))?;
    let (mut writer, reader) = socket.split();
    let (tx, mut rx) = mpsc::channel(128);
    tokio::spawn(async move {
        while let Some(message) = rx.recv().await {
            if writer.send(message).await.is_err() {
                break;
            }
        }
    });
    Ok((tx, reader))
}

async fn handle_browser_signal(
    state: &Arc<AppState>,
    site: &str,
    rtc_session_id: &str,
    pc: &PeerConnection,
    remote_sender_id: &Arc<RwLock<Option<String>>>,
    pending_remote_candidates: &mut Vec<IceCandidate>,
    signal: IncomingRtcSignal,
) -> anyhow::Result<()> {
    match signal.kind.as_str() {
        "offer" => {
            *remote_sender_id.write().await = Some(signal.sender_id.clone());
            let sdp = signal
                .payload
                .get("sdp")
                .and_then(Value::as_str)
                .ok_or_else(|| anyhow!("offer missing payload.sdp"))?;
            let offer = SessionDescription::parse(SdpType::Offer, sdp)
                .map_err(|error| anyhow!("invalid rtc offer sdp: {error}"))?;
            pc.set_remote_description(offer)
                .await
                .map_err(|error| anyhow!("set remote offer failed: {error}"))?;
            for candidate in pending_remote_candidates.drain(..) {
                pc.add_ice_candidate(candidate)
                    .map_err(|error| anyhow!("add buffered ice candidate failed: {error}"))?;
            }
            for transceiver in pc.get_transceivers() {
                if transceiver.kind() == rustrtc::MediaKind::Audio {
                    transceiver.set_direction(rustrtc::TransceiverDirection::SendRecv);
                }
            }
            let answer = pc
                .create_answer()
                .await
                .map_err(|error| anyhow!("create answer failed: {error}"))?;
            pc.set_local_description(answer.clone())
                .map_err(|error| anyhow!("set local answer failed: {error}"))?;
            emit_rtc_signal(
                state,
                site,
                rtc_session_id,
                "answer",
                "ssma-webrtc",
                Some(signal.sender_id),
                json!({
                    "type": "answer",
                    "sdp": answer.to_sdp_string(),
                }),
            )
            .map_err(|(status, body)| anyhow!("emit rtc answer failed: {status} {body:?}"))?;
        }
        "candidate" => {
            if !signal.sender_id.is_empty() {
                *remote_sender_id.write().await = Some(signal.sender_id);
            }
            let candidate_sdp = signal
                .payload
                .get("candidate")
                .and_then(Value::as_str)
                .filter(|value| !value.trim().is_empty())
                .ok_or_else(|| anyhow!("candidate missing payload.candidate"))?;
            let candidate = IceCandidate::from_sdp(candidate_sdp)
                .map_err(|error| anyhow!("invalid rtc candidate: {error}"))?;
            if pc.remote_description().is_some() {
                pc.add_ice_candidate(candidate)
                    .map_err(|error| anyhow!("add ice candidate failed: {error}"))?;
            } else {
                pending_remote_candidates.push(candidate);
            }
        }
        _ => {}
    }
    Ok(())
}

async fn forward_browser_audio_track(
    track: Arc<dyn MediaStreamTrack>,
    models_tx: mpsc::Sender<WsMessage>,
) -> anyhow::Result<()> {
    let mut logged_first_frame = false;
    let mut logged_first_pcm = false;
    let mut decoder = OpusDecoderBridge::new()?;
    loop {
        match track.recv().await {
            Ok(MediaSample::Audio(frame)) => {
                if frame.data.is_empty() {
                    continue;
                }
                if !logged_first_frame {
                    logged_first_frame = true;
                    tracing::info!(
                        payload_type = ?frame.payload_type,
                        bytes = frame.data.len(),
                        "received first inbound webrtc audio frame"
                    );
                }
                let pcm = decoder.decode_to_mono_pcm(frame.data.as_ref())?;
                if pcm.is_empty() {
                    continue;
                }
                if !logged_first_pcm {
                    logged_first_pcm = true;
                    tracing::info!(bytes = pcm.len(), "forwarding first decoded pcm chunk to models_local");
                }
                models_tx
                    .send(WsMessage::Binary(pcm))
                    .await
                    .map_err(|_| anyhow!("models_local ws writer closed"))?;
            }
            Ok(_) => {}
            Err(_) => break,
        }
    }
    Ok(())
}

async fn handle_models_local_message(
    state: &Arc<AppState>,
    site: &str,
    audio_session_id: &str,
    rtc_session_id: &str,
    audio_source: &SampleStreamSource,
    output_bytes: &mut Vec<u8>,
    output_mime: &mut String,
    output_encoding: &mut String,
    output_sample_rate: &mut u32,
    next_rtp_timestamp: &mut u32,
    outbound_encoder: &mut OpusEncoderBridge,
    stop_requested: bool,
    logged_first_backend_event: &mut bool,
    text: &str,
) -> anyhow::Result<bool> {
    let message: ModelsLocalWsMessage =
        serde_json::from_str(text).map_err(|error| anyhow!("invalid models_local ws message: {error}"))?;
    let event = match message {
        ModelsLocalWsMessage::SessionEvent { event } => event,
        ModelsLocalWsMessage::SessionSnapshot { .. } | ModelsLocalWsMessage::Pong => return Ok(false),
        ModelsLocalWsMessage::Error { error } => return Err(anyhow!("models_local ws error: {error}")),
    };
    if !*logged_first_backend_event {
        *logged_first_backend_event = true;
        tracing::info!(
            event_type = %event.event_type,
            status = ?event.status,
            "received first models_local live audio event"
        );
    }
    match event.event_type.as_str() {
        "audio_out_started" => {
            output_bytes.clear();
            output_mime.clear();
            output_encoding.clear();
            output_mime.push_str(event.mime_type.as_deref().unwrap_or("audio/wav"));
            output_encoding.push_str(event.encoding.as_deref().unwrap_or_default());
            *output_sample_rate = event.sample_rate.unwrap_or(RTC_AUDIO_SAMPLE_RATE);
        }
        "audio_out_chunk" => {
            if let Some(base64_chunk) = &event.audio_chunk_base64 {
                let bytes = base64::engine::general_purpose::STANDARD
                    .decode(base64_chunk.as_bytes())
                    .map_err(|error| anyhow!("invalid audio output chunk: {error}"))?;
                output_bytes.extend_from_slice(&bytes);
            }
        }
        "audio_out_stopped" => {
            if !output_bytes.is_empty() {
                stream_audio_bytes_over_webrtc(
                    audio_source,
                    output_bytes,
                    if output_mime.is_empty() {
                        "audio/wav"
                    } else {
                        output_mime.as_str()
                    },
                    if output_encoding.is_empty() {
                        None
                    } else {
                        Some(output_encoding.as_str())
                    },
                    *output_sample_rate,
                    next_rtp_timestamp,
                    outbound_encoder,
                )
                .await?;
                output_bytes.clear();
            }
            output_mime.clear();
            output_encoding.clear();
            *output_sample_rate = RTC_AUDIO_SAMPLE_RATE;
            if stop_requested {
                return Ok(true);
            }
        }
        _ => {}
    }

    apply_audio_backend_event(
        state,
        site,
        audio_session_id,
        &format!("audio.session.{}", event.event_type),
        json!({
            "status": event.status,
            "text": event.text,
            "mimeType": event.mime_type,
            "encoding": event.encoding,
            "sampleRate": event.sample_rate,
            "audioChunkBase64": event.audio_chunk_base64,
            "sequence": event.sequence,
            "finalSegment": event.final_segment,
            "createdAt": event.created_at,
            "rtcSessionId": rtc_session_id,
        }),
    )
    .map_err(|(status, body)| anyhow!("apply audio backend event failed: {status} {body:?}"))?;

    if event.event_type == "ended" && stop_requested && output_bytes.is_empty() {
        return Ok(true);
    }
    Ok(false)
}

async fn stream_audio_bytes_over_webrtc(
    audio_source: &SampleStreamSource,
    audio_bytes: &[u8],
    mime_type: &str,
    encoding: Option<&str>,
    sample_rate: u32,
    next_rtp_timestamp: &mut u32,
    outbound_encoder: &mut OpusEncoderBridge,
) -> anyhow::Result<()> {
    if matches!(encoding, Some("pcm_s16le")) || mime_type.eq_ignore_ascii_case("audio/pcm") {
        let pcm = pcm_bytes_to_best_mono(audio_bytes, sample_rate)?;
        return stream_pcm_over_webrtc(
            audio_source,
            &pcm,
            sample_rate,
            next_rtp_timestamp,
            outbound_encoder,
        )
        .await;
    }
    let pcm = decode_wav_to_mono_pcm(audio_bytes, RTC_AUDIO_SAMPLE_RATE)?;
    stream_pcm_over_webrtc(
        audio_source,
        &pcm,
        RTC_AUDIO_SAMPLE_RATE,
        next_rtp_timestamp,
        outbound_encoder,
    )
    .await
}

async fn stream_pcm_over_webrtc(
    audio_source: &SampleStreamSource,
    pcm: &[i16],
    sample_rate: u32,
    next_rtp_timestamp: &mut u32,
    outbound_encoder: &mut OpusEncoderBridge,
) -> anyhow::Result<()> {
    let resampled = if sample_rate == RTC_AUDIO_SAMPLE_RATE {
        pcm.to_vec()
    } else {
        resample_mono_pcm(pcm, sample_rate, RTC_AUDIO_SAMPLE_RATE)
    };
    for chunk in resampled.chunks(OPUS_FRAME_SAMPLES_PER_CHANNEL) {
        let mut frame = chunk.to_vec();
        if frame.len() < OPUS_FRAME_SAMPLES_PER_CHANNEL {
            frame.resize(OPUS_FRAME_SAMPLES_PER_CHANNEL, 0);
        }
        let stereo = upmix_mono_to_stereo(&frame);
        let encoded = outbound_encoder.encode_stereo_frame(&stereo)?;
        let frame = AudioFrame {
            rtp_timestamp: *next_rtp_timestamp,
            clock_rate: RTC_AUDIO_SAMPLE_RATE,
            data: Bytes::from(encoded),
            payload_type: Some(111),
            marker: false,
            ..Default::default()
        };
        audio_source
            .send(MediaSample::Audio(frame))
            .await
            .map_err(|error| anyhow!("send remote audio frame failed: {error}"))?;
        *next_rtp_timestamp =
            next_rtp_timestamp.wrapping_add(OPUS_FRAME_SAMPLES_PER_CHANNEL as u32);
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    Ok(())
}

fn decode_wav_to_mono_pcm(wav_bytes: &[u8], target_sample_rate: u32) -> anyhow::Result<Vec<i16>> {
    let cursor = std::io::Cursor::new(wav_bytes.to_vec());
    let mut reader = WavReader::new(cursor).map_err(|error| anyhow!("invalid wav output: {error}"))?;
    let spec = reader.spec();
    let channels = usize::from(spec.channels.max(1));
    let mut mono = Vec::new();
    match spec.sample_format {
        hound::SampleFormat::Int => {
            let mut frame = Vec::with_capacity(channels);
            for sample in reader.samples::<i16>() {
                frame.push(sample.map_err(|error| anyhow!("wav sample decode failed: {error}"))?);
                if frame.len() == channels {
                    let sum: i32 = frame.iter().map(|value| i32::from(*value)).sum();
                    mono.push((sum / channels as i32) as i16);
                    frame.clear();
                }
            }
        }
        hound::SampleFormat::Float => {
            let mut frame = Vec::with_capacity(channels);
            for sample in reader.samples::<f32>() {
                frame.push(sample.map_err(|error| anyhow!("wav sample decode failed: {error}"))?);
                if frame.len() == channels {
                    let sum: f32 = frame.iter().sum();
                    mono.push(((sum / channels as f32).clamp(-1.0, 1.0) * i16::MAX as f32) as i16);
                    frame.clear();
                }
            }
        }
    }
    if spec.sample_rate == target_sample_rate {
        return Ok(mono);
    }
    Ok(resample_mono_pcm(&mono, spec.sample_rate, target_sample_rate))
}

fn resample_mono_pcm(samples: &[i16], source_rate: u32, target_rate: u32) -> Vec<i16> {
    if samples.is_empty() || source_rate == 0 {
        return Vec::new();
    }
    if source_rate == target_rate {
        return samples.to_vec();
    }
    if samples.len() == 1 {
        return vec![samples[0]];
    }

    let ratio = f64::from(source_rate) / f64::from(target_rate);
    let output_len =
        ((samples.len() as f64) * f64::from(target_rate) / f64::from(source_rate)).round() as usize;
    let mut resampled = Vec::with_capacity(output_len.max(1));
    for index in 0..output_len.max(1) {
        let src_pos = (index as f64) * ratio;
        let left = src_pos.floor() as usize;
        let frac = src_pos - left as f64;
        let right = (left + 1).min(samples.len().saturating_sub(1));
        let left_sample = f64::from(samples[left]);
        let right_sample = f64::from(samples[right]);
        let value = left_sample + (right_sample - left_sample) * frac;
        let clipped = value.clamp(f64::from(i16::MIN), f64::from(i16::MAX)) as i16;
        resampled.push(clipped);
    }
    resampled
}

fn downmix_stereo_to_mono(samples: &[i16]) -> Vec<i16> {
    samples
        .chunks(2)
        .map(|chunk| {
            let left = i32::from(*chunk.first().unwrap_or(&0));
            let right = i32::from(*chunk.get(1).unwrap_or(chunk.first().unwrap_or(&0)));
            ((left + right) / 2) as i16
        })
        .collect()
}

fn upmix_mono_to_stereo(samples: &[i16]) -> Vec<i16> {
    let mut stereo = Vec::with_capacity(samples.len() * 2);
    for sample in samples {
        stereo.push(*sample);
        stereo.push(*sample);
    }
    stereo
}

fn pcm_bytes_to_samples(bytes: &[u8]) -> anyhow::Result<Vec<i16>> {
    if bytes.len() % 2 != 0 {
        return Err(anyhow!("pcm payload length must be even"));
    }
    Ok(bytes
        .chunks_exact(2)
        .map(|chunk| i16::from_le_bytes([chunk[0], chunk[1]]))
        .collect())
}

fn pcm_bytes_to_best_mono(bytes: &[u8], sample_rate: u32) -> anyhow::Result<Vec<i16>> {
    let samples = pcm_bytes_to_samples(bytes)?;
    let _ = sample_rate;
    Ok(samples)
}

struct OpusDecoderBridge {
    decoder: VoxOpusDecoder,
}

unsafe impl Send for OpusDecoderBridge {}

impl OpusDecoderBridge {
    fn new() -> anyhow::Result<Self> {
        let decoder = OpusCodec::new_decoder(RTC_AUDIO_SAMPLE_RATE as usize, RTC_AUDIO_CHANNELS)
            .map_err(|error| anyhow!("create opus decoder failed: {error}"))?;
        Ok(Self { decoder })
    }

    fn decode_to_mono_pcm(&mut self, payload: &[u8]) -> anyhow::Result<Vec<u8>> {
        if payload.is_empty() {
            return Ok(Vec::new());
        }
        let decoded = self
            .decoder
            .decode::<i16>(Some(payload), OPUS_FRAME_SAMPLES_PER_CHANNEL)
            .map_err(|error| anyhow!("opus decode failed: {error}"))?;
        if decoded.is_empty() {
            return Ok(Vec::new());
        }
        let mono = downmix_stereo_to_mono(&decoded);
        let mut bytes = Vec::with_capacity(mono.len() * 2);
        for sample in mono {
            bytes.extend_from_slice(&sample.to_le_bytes());
        }
        Ok(bytes)
    }
}

struct OpusEncoderBridge {
    encoder: VoxOpusEncoder,
}

unsafe impl Send for OpusEncoderBridge {}

impl OpusEncoderBridge {
    fn new() -> anyhow::Result<Self> {
        let encoder = OpusCodec::new_encoder(
            RTC_AUDIO_SAMPLE_RATE as usize,
            RTC_AUDIO_CHANNELS,
            OpusApplication::Voip,
        )
        .map_err(|error| anyhow!("create opus encoder failed: {error}"))?;
        Ok(Self { encoder })
    }

    fn encode_stereo_frame(&mut self, stereo_pcm: &[i16]) -> anyhow::Result<Vec<u8>> {
        let frame_size = stereo_pcm.len() / RTC_AUDIO_CHANNELS;
        let encoded = self
            .encoder
            .encode(stereo_pcm, frame_size, OpusCodec::MAX_PACKET_SIZE)
            .map_err(|error| anyhow!("opus encode failed: {error}"))?;
        if encoded.is_empty() {
            return Err(anyhow!("opus encode failed: empty packet"));
        }
        Ok(encoded)
    }
}
