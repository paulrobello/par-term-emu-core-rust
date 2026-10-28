/* Microphone STATUS for the Par-mux.app launcher (app/launcher.c): AVFoundation
   is Obj-C, so the read lives here behind a C entry point. Status-only — the
   REQUEST must never be made from the launcher: TCC aborts the process with
   TCC_CRASHING_DUE_TO_PRIVACY_VIOLATION when the responsible bundle lacks
   NSMicrophoneUsageDescription (2026-10-27: every pane-launched start died
   "Abort trap: 6" before exec'ing the daemon), and under pane/GUI attribution
   the responsible bundle is par-term.app, which has no such key. notDetermined
   returns -1 and is left that way — the pane's own app requests mic access
   when it actually needs it. */
#import <AVFoundation/AVFoundation.h>

int par_mux_microphone_status(void) {
    AVAuthorizationStatus status =
        [AVCaptureDevice authorizationStatusForMediaType:AVMediaTypeAudio];
    return status == AVAuthorizationStatusNotDetermined ? -1
         : status == AVAuthorizationStatusAuthorized;
}
