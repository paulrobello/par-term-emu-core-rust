/* Microphone access for the Par-mux.app launcher (app/launcher.c): AVFoundation
   is Obj-C, so the request lives here behind a C entry point. NotDetermined asks
   and returns -1 — the system owns the prompt and the answer lands in TCC for
   the bundle even though the launcher execs into the daemon before it is
   answered. A settled status returns 1 (authorized) or 0. */
#import <AVFoundation/AVFoundation.h>

int par_mux_request_microphone_access(void) {
    AVMediaType audio = AVMediaTypeAudio;
    AVAuthorizationStatus status = [AVCaptureDevice authorizationStatusForMediaType:audio];
    if (status != AVAuthorizationStatusNotDetermined)
        return status == AVAuthorizationStatusAuthorized;
    [AVCaptureDevice requestAccessForMediaType:audio
                             completionHandler:^(BOOL granted) {}];
    return -1;
}
