import React from 'react';
import {Composition, registerRoot} from 'remotion';
import {VakArchitectureVideo} from './VakArchitectureVideo';

export const RemotionRoot: React.FC = () => {
  return (
    <Composition
      id="VakArchitecture"
      component={VakArchitectureVideo}
      durationInFrames={90 * 30}
      fps={30}
      width={1920}
      height={1080}
      defaultProps={{
        voiceoverSrc: null,
        musicSrc: null,
        captionsSrc: null,
      }}
    />
  );
};

registerRoot(RemotionRoot);
