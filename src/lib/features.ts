// Features that the app hides while their design is not complete.
//
// Computer audio stays hidden until the processing of a stereo note is
// designed again. The backend has the same switch
// (`SYSTEM_AUDIO_AVAILABLE`) and refuses to turn Computer audio on. The
// tests of the hidden controls set `systemAudio` to true.
export const features = {
  systemAudio: false,
};
