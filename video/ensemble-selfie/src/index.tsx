import React from 'react';
import {Composition, registerRoot} from 'remotion';
import {EnsembleSelfie} from './EnsembleSelfie';

const Root: React.FC = () => <Composition id="EnsembleSelfie" component={EnsembleSelfie} durationInFrames={720} fps={30} width={1280} height={720}/>;
registerRoot(Root);
