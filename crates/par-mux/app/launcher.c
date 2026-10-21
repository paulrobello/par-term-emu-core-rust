/* Par-mux.app launcher: execs the par-mux binary in-process so macOS TCC
   attributes the daemon — and every pane child — to this app bundle. A bare CLI
   daemon has no grantable identity, so screen capture, microphone, and local
   network sockets are silently blocked with no prompt ever appearing.

   Bare launch runs `par-mux --foreground --no-attach` (the launchd shape);
   arguments are forwarded verbatim, making the bundle binary a faithful
   par-mux client proxy for prompt-trigger tests (`Par-mux status`) and
   scripted callers. stdout/stderr append to ~/Library/Logs/par-mux/daemon.log
   because launchd/Finder give the app no terminal.

   Before exec it fires the TCC prompts pane children inherit from the bundle:
   Screen Recording (screencapture), Accessibility (keyboard/event control),
   Microphone, and Local Network. Screen Recording and Accessibility have no
   Info.plist usage key — macOS only prompts when something calls the request
   API, so the launcher is that something. Each request is preflighted (silent
   once granted) and the prompts are presented and owned by the system, so they
   outlive this process exec'ing into the daemon. Full Disk Access, Developer
   Tools, and per-target Automation have no prompt API and stay manual grants
   (docs/MUX.md "macOS app bundle"). */
#include <ApplicationServices/ApplicationServices.h>
#include <CoreGraphics/CoreGraphics.h>
#include <arpa/inet.h>
#include <errno.h>
#include <fcntl.h>
#include <netinet/in.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/socket.h>
#include <sys/stat.h>
#include <unistd.h>

/* Microphone is requestable only through AVFoundation (Obj-C): app/permissions.m. */
extern int par_mux_request_microphone_access(void);

/* Local Network has no preflight or status API: the prompt fires the first time
   the bundle's own binary touches the LAN through a CONNECT — a bare UDP sendto
   does not trip the check (measured: datagrams leave silently, no dialog). One
   short non-blocking TCP connect to the default gateway is the verified trigger
   shape (the LanProbe/herdr evidence was all connects); granted or denied it
   stays silent, only notDetermined presents. Best-effort: without a parseable
   gateway the first pane-child LAN use fires the prompt instead (attribution
   to the bundle covers pane children either way). */
static int request_local_network_access(void) {
    FILE *route = popen("/sbin/route -n get default 2>/dev/null", "r");
    if (!route) return 0;
    char line[256], gw[64] = "";
    while (fgets(line, sizeof line, route)) {
        if (sscanf(line, " gateway: %63s", gw) == 1) break;
    }
    pclose(route);
    if (!gw[0]) return 0;

    int fd = socket(AF_INET, SOCK_STREAM, 0);
    if (fd < 0) return 0;
    struct sockaddr_in dst;
    memset(&dst, 0, sizeof dst);
    dst.sin_family = AF_INET;
    dst.sin_port = htons(80);
    if (inet_pton(AF_INET, gw, &dst.sin_addr) != 1) {
        close(fd);
        return 0;
    }
    fcntl(fd, F_SETFL, O_NONBLOCK);
    int rc = connect(fd, (struct sockaddr *)&dst, sizeof dst);
    if (rc != 0 && (errno == EINPROGRESS || errno == EWOULDBLOCK)) {
        fd_set writers;
        FD_ZERO(&writers);
        FD_SET(fd, &writers);
        struct timeval tv = {1, 0};
        select(fd + 1, NULL, &writers, NULL, &tv);
    }
    close(fd);
    return 1; /* the attempt itself trips the check; the outcome is irrelevant */
}

static void request_permissions(void) {
    int screen = CGPreflightScreenCaptureAccess();
    if (!screen)
        CGRequestScreenCaptureAccess();

    /* The prompt option is ignored when already trusted: one call covers both. */
    CFTypeRef keys = kAXTrustedCheckOptionPrompt;
    CFTypeRef vals = kCFBooleanTrue;
    CFDictionaryRef opts = CFDictionaryCreate(
        NULL, &keys, &vals, 1, &kCFTypeDictionaryKeyCallBacks, &kCFTypeDictionaryValueCallBacks);
    int accessibility = AXIsProcessTrustedWithOptions(opts);
    CFRelease(opts);

    int microphone = par_mux_request_microphone_access();
    int local_network = request_local_network_access();
    fprintf(stderr, "par-mux-app: permissions screen=%s accessibility=%s microphone=%s local_network=%s\n",
            screen ? "granted" : "asked",
            accessibility ? "granted" : "asked",
            microphone < 0 ? "asked" : (microphone ? "granted" : "denied"),
            local_network ? "probed" : "skipped");
}

static const char *par_mux_bin(void) {
    const char *env = getenv("PAR_MUX_BIN");
    if (env && *env) return env;
    const char *home = getenv("HOME");
    static char path[512];
    if (home) {
        snprintf(path, sizeof path, "%s/.cargo/bin/par-mux", home);
        if (access(path, X_OK) == 0) return path;
    }
    return "/usr/local/bin/par-mux";
}

int main(int argc, char **argv) {
    const char *home = getenv("HOME");
    char logdir[512], logpath[560];
    snprintf(logdir, sizeof logdir, "%s/Library/Logs/par-mux", home ? home : "/tmp");
    mkdir(logdir, 0755); /* EEXIST is fine */
    snprintf(logpath, sizeof logpath, "%s/daemon.log", logdir);
    int log = open(logpath, O_WRONLY | O_CREAT | O_APPEND, 0644);
    if (log >= 0) {
        dup2(log, STDOUT_FILENO);
        dup2(log, STDERR_FILENO);
    }

    request_permissions();

    const char *bin = par_mux_bin();
    char *exec_args[255];
    int n = 0;
    exec_args[n++] = "par-mux";
    if (argc == 1) {
        /* --foreground serves in this process (launchd supervises it) and skips
           the auto-attach a daemonized start would attempt. */
        exec_args[n++] = "--foreground";
    } else {
        for (int i = 1; i < argc && n < 253; i++) exec_args[n++] = argv[i];
    }
    exec_args[n] = NULL;
    execv(bin, exec_args);
    perror("par-mux-app: exec");
    return 1;
}
