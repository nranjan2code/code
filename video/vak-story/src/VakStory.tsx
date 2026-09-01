import React from 'react';
import {AbsoluteFill, Audio, Img, interpolate, Sequence, spring, staticFile, useCurrentFrame, useVideoConfig} from 'remotion';
import {assetPath} from './assetManifest';
import {beats, type StoryBeat} from './storyData';

type Props = {voiceoverSrc: string | null; musicSrc: string | null};
const C = {bg: '#171714', surface: '#22221f', text: '#eeeae2', soft: '#c4c0b8', muted: '#918e86', border: '#34342f', accent: '#df795f'};

const Grain: React.FC = () => (
  <AbsoluteFill style={{pointerEvents: 'none', opacity: 0.08, mixBlendMode: 'screen', backgroundImage: 'url("data:image/svg+xml,%3Csvg viewBox=\'0 0 180 180\' xmlns=\'http://www.w3.org/2000/svg\'%3E%3Cfilter id=\'n\'%3E%3CfeTurbulence type=\'fractalNoise\' baseFrequency=\'.8\' numOctaves=\'4\' stitchTiles=\'stitch\'/%3E%3C/filter%3E%3Crect width=\'100%25\' height=\'100%25\' filter=\'url(%23n)\' opacity=\'.35\'/%3E%3C/svg%3E")'}} />
);

const Progress: React.FC<{index: number}> = ({index}) => (
  <div style={{position: 'absolute', left: 88, right: 88, bottom: 48, display: 'flex', gap: 7}}>
    {beats.map((beat, i) => <div key={beat.id} style={{height: 3, flex: 1, background: i <= index ? C.accent : C.border, opacity: i === index ? 1 : 0.7}} />)}
  </div>
);

const ImagePanel: React.FC<{id: string; frame: number; tint: string}> = ({id, frame, tint}) => {
  const zoom = interpolate(frame, [0, 300], [1.04, 1.11], {extrapolateRight: 'clamp'});
  const drift = interpolate(frame, [0, 300], [0, -18], {extrapolateRight: 'clamp'});
  return <div style={{position: 'absolute', right: 92, top: 176, width: 850, height: 610, overflow: 'hidden', border: `1px solid ${C.border}`, background: C.surface}}>
    <Img src={staticFile(assetPath(id))} alt="" style={{width: '100%', height: '100%', objectFit: 'cover', transform: `scale(${zoom}) translateY(${drift}px)`, filter: 'saturate(.78) contrast(1.04)'}} />
    <div style={{position: 'absolute', inset: 0, background: `linear-gradient(90deg, ${C.bg} 0%, transparent 42%), linear-gradient(0deg, rgba(23,23,20,.62), transparent 35%)`}} />
    <div style={{position: 'absolute', left: 24, top: 24, color: tint, fontSize: 16, letterSpacing: 2, textTransform: 'uppercase'}}>evidence / visual reference</div>
  </div>;
};

const Beat: React.FC<{beat: StoryBeat; index: number}> = ({beat, index}) => {
  const frame = useCurrentFrame();
  const {fps} = useVideoConfig();
  const enter = spring({frame, fps, config: {damping: 22, stiffness: 90}});
  const line = interpolate(enter, [0, 1], [80, 0]);
  const isOpening = !beat.asset;
  return <AbsoluteFill style={{background: C.bg, color: C.text, fontFamily: "-apple-system, BlinkMacSystemFont, 'SF Pro Text', Inter, sans-serif"}}>
    <div style={{position: 'absolute', inset: 0, background: `radial-gradient(circle at ${isOpening ? '50%' : '76%'} 44%, ${beat.tint}22, transparent 34%)`}} />
    {beat.asset ? <ImagePanel id={beat.asset} frame={frame} tint={beat.tint} /> : null}
    <div style={{position: 'absolute', left: 92, top: isOpening ? 300 : 205, width: beat.asset ? 780 : 1250, transform: `translateY(${line}px)`, opacity: enter}}>
      <div style={{fontSize: 17, letterSpacing: 3, color: beat.tint, textTransform: 'uppercase', marginBottom: 28}}>{beat.eyebrow}</div>
      <h1 style={{fontSize: isOpening ? 104 : 72, lineHeight: 1.02, letterSpacing: '-0.035em', margin: 0, maxWidth: beat.asset ? 730 : 1200, fontWeight: 650}}>{beat.title}</h1>
      <p style={{fontSize: 29, lineHeight: 1.32, color: C.soft, maxWidth: beat.asset ? 650 : 800, marginTop: 32, marginBottom: 0}}>{beat.line}</p>
    </div>
    {isOpening ? <div style={{position: 'absolute', right: 110, bottom: 170, color: C.muted, fontFamily: 'SFMono-Regular, ui-monospace, monospace', fontSize: 17}}>request_id: waiting_for_permission</div> : null}
    {!isOpening ? <div style={{position: 'absolute', left: 92, bottom: 118, color: C.muted, fontFamily: 'SFMono-Regular, ui-monospace, monospace', fontSize: 16}}>vak / {String(index).padStart(2, '0')} / {beat.id}</div> : null}
    <Progress index={index} />
  </AbsoluteFill>;
};

export const VakStory: React.FC<Props> = ({voiceoverSrc, musicSrc}) => {
  const {fps} = useVideoConfig();
  const beatDuration = 150;
  return <AbsoluteFill>
    {beats.map((beat, index) => <Sequence key={beat.id} from={index * beatDuration} durationInFrames={beatDuration}><Beat beat={beat} index={index} /></Sequence>)}
    {voiceoverSrc ? <Audio src={voiceoverSrc.startsWith('http') ? voiceoverSrc : staticFile(voiceoverSrc)} volume={1} /> : null}
    {musicSrc ? <Audio src={musicSrc.startsWith('http') ? musicSrc : staticFile(musicSrc)} volume={0.1} loop /> : null}
    <Grain />
  </AbsoluteFill>;
};
