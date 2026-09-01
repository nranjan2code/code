import React from 'react';
import {Composition, registerRoot} from 'remotion';
import {VakStory} from './VakStory';

export const RemotionRoot: React.FC = () => (
  <Composition
    id="VakStory"
    component={VakStory}
    durationInFrames={90 * 30}
    fps={30}
    width={1920}
    height={1080}
    defaultProps={{voiceoverSrc: null, musicSrc: null}}
  />
);

registerRoot(RemotionRoot);
