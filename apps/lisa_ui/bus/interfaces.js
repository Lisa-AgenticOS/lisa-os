// GENERATED from apps/lisa_ui/bus/xml/ — edit those (or re-introspect
// the daemons), then run python3 os/repo-tools/build-bus-interfaces.py.
// Hand edits are overwritten.
//
// The sdk's ONE copy of the system D-Bus interfaces (ADR-0060). The
// xml/ snapshots are introspected from the RUNNING daemons on the
// reference device, so this file describes what the daemons serve, not
// what a surface once believed they served.

export const IFACE_XML = {
    Agent1: `
<node>
<interface name="dev.lisaos.Agent1">
    <!--
     Liveness probe.
     -->
    <method name="Ping">
      <arg type="s" direction="out"/>
    </method>
    <!--
     All registered tools as a JSON array
     (\`[{app_id, name, tier, description, undoable}]\`).
     -->
    <method name="ListTools">
      <arg type="s" direction="out"/>
    </method>
    <!--
     Discovery: rank tools against a natural-language query.
     -->
    <method name="Discover">
      <arg name="query" type="s" direction="in"/>
      <arg type="s" direction="out"/>
    </method>
    <!--
     Request a tool call. Read-tier calls with a fully trusted chain
     execute immediately; everything else parks and emits
     ConfirmationRequested (answer via Confirm). Every path is
     ledgered before anything happens.
     -->
    <method name="RequestCall">
      <arg name="app_id" type="s" direction="in"/>
      <arg name="tool" type="s" direction="in"/>
      <arg name="args_json" type="s" direction="in"/>
      <arg name="options" type="a{sv}" direction="in"/>
      <arg type="t" direction="out"/>
      <arg type="s" direction="out"/>
      <arg type="s" direction="out"/>
    </method>
    <!--
     Answer a pending confirmation. Status: "executed" | "failed" |
     "denied".
     -->
    <method name="Confirm">
      <arg name="call_id" type="t" direction="in"/>
      <arg name="approve" type="b" direction="in"/>
      <arg type="s" direction="out"/>
      <arg type="s" direction="out"/>
    </method>
    <!--
     Revert the caller's last agent action via its journaled
     compensation.

     The identity comes from the transport, never from the message
     (ADR-0033). This method used to take no arguments at all and
     hardcode the actor \`"host"\`, so any peer on the session bus could
     revert any other peer's action and the Ledger would attribute it
     to "host" (#94).
     -->
    <method name="Undo">
      <arg type="s" direction="out"/>
    </method>
    <!--
     Emitted when a call parks for confirmation; \`spec_json\` carries
     the typed-diff material (tool, args, tiers, escalation, chain).
     -->
    <signal name="ConfirmationRequested">
      <arg name="call_id" type="t"/>
      <arg name="spec_json" type="s"/>
    </signal>
    <!--
     Emitted when a call is REFUSED (#251). There is no parked call
     behind it and no \`Confirm\` that can answer it: \`report_json\` is
     for a dialog that reports, with one button and no approving
     control. It carries no arguments and no command, so nothing
     downstream can rebuild the refused action from it.
     -->
    <signal name="RefusalReported">
      <arg name="call_id" type="t"/>
      <arg name="report_json" type="s"/>
    </signal>
  </interface>
</node>`,
    Context1: `
<node>
<interface name="dev.lisaos.Context1">
    <!--
     Liveness probe.
     -->
    <method name="Ping">
      <arg type="s" direction="out"/>
    </method>
    <!--
     Retrieval over the user's index. Options: "limit" (u, default
     3), "hybrid" (b, BM25×cosine blend), "scopes" (as — when
     present, the ACL-scoped path: only provenance the granted
     scopes permit is ever returned, deny-by-default). Returns a
     JSON array of hits. The ledger entry is appended before the
     store is touched; append failure refuses the search.
     -->
    <method name="Search">
      <arg name="query" type="s" direction="in"/>
      <arg name="options" type="a{sv}" direction="in"/>
      <arg type="s" direction="out"/>
    </method>
    <!--
     Read one key from an app's memory namespace. A missing key is
     an error (mirrors \`lisa memory get\`).
     -->
    <method name="MemoryGet">
      <arg name="app" type="s" direction="in"/>
      <arg name="key" type="s" direction="in"/>
      <arg type="s" direction="out"/>
    </method>
    <!--
     Upsert one key in an app's memory namespace.
     -->
    <method name="MemorySet">
      <arg name="app" type="s" direction="in"/>
      <arg name="key" type="s" direction="in"/>
      <arg name="value" type="s" direction="in"/>
    </method>
    <!--
     All keys in an app's namespace as a JSON object (key → value).
     -->
    <method name="MemoryList">
      <arg name="app" type="s" direction="in"/>
      <arg type="s" direction="out"/>
    </method>
    <!--
     Wipe an app's namespace entirely (zero residual rows, §5.3).
     -->
    <method name="MemoryWipe">
      <arg name="app" type="s" direction="in"/>
    </method>
  </interface>
</node>`,
    Harness1: `
<node>
<interface name="dev.lisaos.Harness1">
    <method name="Ping">
      <arg type="s" direction="out"/>
    </method>
    <!--
     Start a run. Returns immediately with an id; progress arrives as
     signals, so a frontend stays responsive and can Cancel.
     -->
    <method name="Run">
      <arg name="prompt" type="s" direction="in"/>
      <arg name="options" type="a{sv}" direction="in"/>
      <arg type="t" direction="out"/>
    </method>
    <!--
     Ask a run to stop. It finishes the turn already in flight — a
     tool call killed halfway is how half-done actions happen.
     -->
    <method name="Cancel">
      <arg name="run_id" type="t" direction="in"/>
    </method>
    <signal name="Tool">
      <arg name="run_id" type="t"/>
      <arg name="name" type="s"/>
      <arg name="detail" type="s"/>
    </signal>
    <signal name="Token">
      <arg name="run_id" type="t"/>
      <arg name="delta" type="s"/>
    </signal>
    <signal name="Finished">
      <arg name="run_id" type="t"/>
      <arg name="ok" type="b"/>
      <arg name="summary" type="s"/>
    </signal>
  </interface>
</node>`,
    Inference1: `
<node>
<interface name="dev.lisaos.Inference1">
    <!--
     Liveness probe.
     -->
    <method name="Ping">
      <arg type="s" direction="out"/>
    </method>
    <!--
     Open a session. Returns the session object path and the read end
     of the token pipe. Options: "model_hint" (s) selects a resident
     model; memory_ns and scopes arrive with the portal (M2).
     -->
    <method name="OpenSession">
      <arg name="options" type="a{sv}" direction="in"/>
      <arg type="o" direction="out"/>
      <arg type="h" direction="out"/>
    </method>
  </interface>
</node>`,
    Overlay1: `
<node>
<interface name="dev.lisaos.Overlay1">
    <method name="Ask">
      <arg type="s" name="prompt" direction="in">
      </arg>
      <arg type="a{sv}" name="options" direction="in">
      </arg>
      <arg type="t" name="query_id" direction="out">
      </arg>
    </method>
    <method name="Cancel">
      <arg type="t" name="query_id" direction="in">
      </arg>
    </method>
    <method name="Respond">
      <arg type="t" name="query_id" direction="in">
      </arg>
      <arg type="b" name="approve" direction="in">
      </arg>
    </method>
    <method name="GetStatus">
      <arg type="a{sv}" name="status" direction="out">
      </arg>
    </method>
    <signal name="Started">
      <arg type="t" name="query_id">
      </arg>
      <arg type="s" name="meta_json">
      </arg>
    </signal>
    <signal name="Token">
      <arg type="t" name="query_id">
      </arg>
      <arg type="s" name="text">
      </arg>
    </signal>
    <signal name="ConfirmationNeeded">
      <arg type="t" name="query_id">
      </arg>
      <arg type="s" name="spec_json">
      </arg>
    </signal>
    <signal name="Finished">
      <arg type="t" name="query_id">
      </arg>
      <arg type="s" name="status">
      </arg>
      <arg type="s" name="detail">
      </arg>
    </signal>
  </interface>
</node>`,
    Remote1: `
<node>
<interface name="dev.lisaos.Remote1">
    <!--
     Liveness probe.
     -->
    <method name="Ping">
      <arg type="s" direction="out"/>
    </method>
    <!--
     Providers + credential presence + consent, one JSON document —
     the Settings page renders straight from this.
     -->
    <method name="State">
      <arg type="s" direction="out"/>
    </method>
    <!--
     Register a user-supplied OpenAI-compatible endpoint (§5.11).

     Public-internet rules only. An endpoint on this machine or this
     LAN is a deliberate, explained choice (#92), and Settings has no
     UI for that question — so it is made where the question can
     actually be asked: \`lisa remote add --allow-local\`.
     -->
    <method name="AddProvider">
      <arg name="id" type="s" direction="in"/>
      <arg name="display_name" type="s" direction="in"/>
      <arg name="base_url" type="s" direction="in"/>
    </method>
    <method name="RemoveProvider">
      <arg name="id" type="s" direction="in"/>
    </method>
    <!--
     Store a credential. Write-only: no method returns key material.
     -->
    <method name="SetKey">
      <arg name="id" type="s" direction="in"/>
      <arg name="key" type="s" direction="in"/>
    </method>
    <method name="ClearKey">
      <arg name="id" type="s" direction="in"/>
    </method>
    <!--
     Flip a per-scope "may offload" switch (default: nothing leaves).
     -->
    <method name="SetConsent">
      <arg name="scope" type="s" direction="in"/>
      <arg name="allowed" type="b" direction="in"/>
    </method>
    <!--
     Begin "Sign in with …" for an OAuth-capable provider (\`anthropic\`
     or \`openai\`); returns the authorize URL for the panel to open in
     the browser. The broker binds a loopback callback server and does
     the token exchange when the browser redirects back; completion
     arrives asynchronously via the \`LoginCompleted\` signal. Fails for
     key-only providers.
     -->
    <method name="BeginLogin">
      <arg name="provider_id" type="s" direction="in"/>
      <arg type="s" direction="out"/>
    </method>
    <!--
     Forget a stored OAuth session (idempotent).
     -->
    <method name="Logout">
      <arg name="provider_id" type="s" direction="in"/>
    </method>
    <!--
     The provider's live model list (its own \`/models\`), as a JSON array
     of ids — for the Settings model dropdown. Requires a stored key.
     -->
    <method name="ListModels">
      <arg name="provider" type="s" direction="in"/>
      <arg type="s" direction="out"/>
    </method>
    <!--
     Emitted once a \`BeginLogin\` flow finishes: \`ok\` true on a stored
     session, false on error/timeout; \`detail\` is a human-readable
     status. No token material is ever carried.
     -->
    <signal name="LoginCompleted">
      <arg name="provider_id" type="s"/>
      <arg name="ok" type="b"/>
      <arg name="detail" type="s"/>
    </signal>
  </interface>
</node>`,
    Voice1: `
<node>
<interface name="dev.lisaos.Voice1">
    <method name="StartListening">
      <arg type="t" name="session_id" direction="out">
      </arg>
    </method>
    <method name="StopListening">
      <arg type="t" name="session_id" direction="in">
      </arg>
    </method>
    <method name="Cancel">
      <arg type="t" name="session_id" direction="in">
      </arg>
    </method>
    <method name="GetState">
      <arg type="a{sv}" name="state" direction="out">
      </arg>
    </method>
    <signal name="ListeningStarted">
      <arg type="t" name="session_id">
      </arg>
    </signal>
    <signal name="Transcribing">
      <arg type="t" name="session_id">
      </arg>
    </signal>
    <signal name="Transcribed">
      <arg type="t" name="session_id">
      </arg>
      <arg type="s" name="text">
      </arg>
    </signal>
    <signal name="Failed">
      <arg type="t" name="session_id">
      </arg>
      <arg type="s" name="reason">
      </arg>
    </signal>
  </interface>
</node>`,
};
