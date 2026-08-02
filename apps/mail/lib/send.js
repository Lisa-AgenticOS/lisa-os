// Handing a composed message to msmtp, and deciding what happened.
// Pure: argv in, outcome out — no subprocess here, so the failure
// handling is testable without an SMTP server to fail against.
//
// # Why msmtp and not a client of our own
//
// Same reasoning as mbsync for receiving (cli/lisa/src/mail.rs): TLS,
// AUTH mechanisms, XOAUTH2 token refresh and server quirks are a
// decade of other people's bug reports. `lisa mail setup` already
// writes ~/.config/lisa/msmtprc with an account block per identity, so
// the credential story is solved and not re-solved here.
//
// # The rule this module exists to enforce
//
// A send that fails must not lose the message. That is the app's
// version of the defect this repo keeps finding — something reporting
// success while doing nothing — and it is worse here, because the thing
// lost is the user's writing rather than a state flag.

/// The msmtp invocation for one message.
///
/// `-t` reads the recipients from the message's own To/Cc headers, so
/// the address list exists in exactly one place: the message. Passing
/// recipients separately is how a Cc silently goes undelivered while the
/// header still shows it.
///
/// `--read-envelope-from` takes the envelope sender from `From:` for the
/// same reason.
export function msmtpArgv(configPath, account) {
    const argv = ['msmtp', '--file', String(configPath), '-t', '--read-envelope-from'];
    if (account)
        argv.push('-a', String(account));
    return argv;
}

/// What an msmtp run means.
///
/// Exit 0 is delivery accepted by the submission server — not delivery
/// to the recipient, which nobody can promise, and the wording says so
/// rather than claiming more than SMTP does.
///
/// Everything else keeps the message. `retryable` distinguishes "the
/// network was down" from "the server refused this address", because
/// the first is worth a retry button and the second is worth editing
/// the message.
export function sendOutcome(exitStatus, stderr = '') {
    const err = String(stderr ?? '').trim();
    if (exitStatus === 0)
        return {sent: true, message: 'Sent'};

    // msmtp exits 78 (EX_CONFIG) when the account or config is wrong,
    // which is a setup problem the user can act on and no amount of
    // retrying fixes.
    if (exitStatus === 78) {
        return {
            sent: false, retryable: false,
            message: 'No outgoing account is configured — run `lisa mail setup`',
            detail: err,
        };
    }
    // A permanent SMTP refusal (5xx) is quoted back: "550 no such user"
    // tells the sender to fix the address, which "sending failed" does
    // not.
    //
    // MATCHED IN CONTEXT, not as a bare three-digit number. The first
    // version tested /\b5\d\d\b/ against the whole of stderr, and
    //
    //   cannot connect to smtp.example.test, port 587: Network unreachable
    //
    // matched — on the PORT. A network outage was reported to the user
    // as "the server refused this message", which is both wrong and
    // unactionable, and it would have marked a retryable failure
    // permanent. Ports, timestamps and byte counts are all three digits
    // starting with 5; an SMTP reply code is only meaningful where the
    // server is speaking.
    const permanent = /(?:server (?:message|reply|said)[^\n]*?|^)\b5\d\d\b/mi.test(err);
    return {
        sent: false,
        retryable: !permanent,
        message: permanent
            ? `The server refused this message: ${firstLine(err)}`
            : `Could not send: ${firstLine(err) || `msmtp exited ${exitStatus}`}`,
        detail: err,
    };
}

function firstLine(text) {
    return String(text).split('\n').map((s) => s.trim()).filter(Boolean)[0] ?? '';
}

/// The Maildir filename for a message we wrote.
///
/// `S` because the sender has, by definition, seen it. Delivered
/// straight to `cur/`: `new/` means "arrived and untouched", and a copy
/// of your own outgoing mail sitting in `new/` shows up as unread mail
/// from yourself in every client on that Maildir.
export function sentFilename(now, random, host = 'lisa') {
    return `${now}.${random}.${host}:2,S`;
}
