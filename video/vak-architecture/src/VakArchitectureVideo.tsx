import React, {useEffect, useState} from 'react';
import {AbsoluteFill, Audio, Sequence, interpolate, spring, staticFile, useCurrentFrame, useDelayRender, useVideoConfig} from 'remotion';
import type {Scene} from './sceneData';
import {scenes} from './sceneData';

type Caption = {text: string; startMs: number; endMs: number};
type Props = {voiceoverSrc: string | null; musicSrc: string | null; captionsSrc: string | null};

const colors = {ink: '#102544', muted: '#58708d', paper: '#f8f7f2', panel: '#ffffff'};

const NodeRow: React.FC<{scene: Scene; frame: number}> = ({scene, frame}) => {
  return (
    <div style={{display: 'flex', gap: 18, alignItems: 'center', justifyContent: 'center', flexWrap: 'wrap', marginTop: 62}}>
      {scene.nodes.map((node, index) => {
        const delay = 10 + index * 4;
        const y = spring({frame: Math.max(0, frame - delay), fps: 30, config: {damping: 16, stiffness: 120}});
        return (
          <React.Fragment key={node}>
            <div style={{transform: `translateY(${interpolate(y, [0, 1], [40, 0])}px)`, opacity: y, background: colors.panel, border: `2px solid ${scene.accent}`, borderRadius: 18, padding: '18px 25px', minWidth: 150, textAlign: 'center', fontSize: 25, fontWeight: 700, color: colors.ink}}>{node}</div>
            {index < scene.nodes.length - 1 ? <div style={{color: scene.accent, fontSize: 34, opacity: y}}>→</div> : null}
          </React.Fragment>
        );
      })}
    </div>
  );
};

const SceneCard: React.FC<{scene: Scene; index: number}> = ({scene, index}) => {
  const frame = useCurrentFrame();
  const titleProgress = spring({frame, fps: 30, config: {damping: 18, stiffness: 90}});
  const bodyProgress = spring({frame: Math.max(0, frame - 8), fps: 30, config: {damping: 18, stiffness: 90}});
  return (
    <AbsoluteFill style={{background: colors.paper, color: colors.ink, padding: '92px 112px', fontFamily: 'Inter, ui-sans-serif, system-ui, sans-serif'}}>
      <div style={{display: 'flex', justifyContent: 'space-between', alignItems: 'center'}}>
        <div style={{fontSize: 26, fontWeight: 800, letterSpacing: 4, color: scene.accent}}>{scene.kicker}</div>
        <div style={{fontSize: 24, color: colors.muted}}>VAK / ARCHITECTURE</div>
      </div>
      <div style={{height: 7, width: 180, background: scene.accent, borderRadius: 99, marginTop: 28}} />
      <div style={{maxWidth: 1450, transform: `translateY(${interpolate(titleProgress, [0, 1], [60, 0])}px)`, opacity: titleProgress}}>
        <h1 style={{fontSize: 86, lineHeight: 1.03, letterSpacing: -3, margin: '56px 0 30px', maxWidth: 1500}}>{scene.title}</h1>
        <p style={{fontSize: 36, lineHeight: 1.35, color: colors.muted, maxWidth: 1260, margin: 0, opacity: bodyProgress}}>{scene.body}</p>
      </div>
      <NodeRow scene={scene} frame={frame} />
      <div style={{position: 'absolute', left: 112, right: 112, bottom: 75, display: 'flex', justifyContent: 'space-between', color: colors.muted, fontSize: 22}}>
        <span>trust → execution → evidence → delivery</span>
        <span>{String(index + 1).padStart(2, '0')} / {String(scenes.length).padStart(2, '0')}</span>
      </div>
    </AbsoluteFill>
  );
};

const Captions: React.FC<{src: string}> = ({src}) => {
  const frame = useCurrentFrame();
  const {fps} = useVideoConfig();
  const {delayRender, continueRender, cancelRender} = useDelayRender();
  const [captions, setCaptions] = useState<Caption[] | null>(null);
  useEffect(() => {
    const handle = delayRender();
    fetch(staticFile(src))
      .then((response) => response.json())
      .then((value: Caption[]) => { setCaptions(value); continueRender(handle); })
      .catch((error) => cancelRender(error));
  }, [cancelRender, continueRender, delayRender, src]);
  if (!captions) return null;
  const now = (frame / fps) * 1000;
  const text = captions.filter((caption) => caption.startMs <= now && caption.endMs > now).map((caption) => caption.text).join('');
  return text ? <div style={{position: 'absolute', left: 170, right: 170, bottom: 115, textAlign: 'center', color: colors.panel, background: colors.ink, borderRadius: 12, padding: '12px 20px', fontSize: 26, fontWeight: 700}}>{text}</div> : null;
};

const assetSrc = (src: string) => src.startsWith('http') ? src : staticFile(src);

export const VakArchitectureVideo: React.FC<Props> = ({voiceoverSrc, musicSrc, captionsSrc}) => {
  const {fps} = useVideoConfig();
  const sceneDuration = 10 * fps;
  return (
    <AbsoluteFill>
      {scenes.map((scene, index) => (
        <Sequence key={scene.title} from={index * sceneDuration} durationInFrames={sceneDuration}>
          <SceneCard scene={scene} index={index} />
        </Sequence>
      ))}
      {voiceoverSrc ? <Audio src={assetSrc(voiceoverSrc)} volume={1} /> : null}
      {musicSrc ? <Audio src={assetSrc(musicSrc)} volume={0.12} loop /> : null}
      {captionsSrc ? <Captions src={captionsSrc} /> : null}
    </AbsoluteFill>
  );
};
