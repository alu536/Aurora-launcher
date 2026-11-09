import { useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { check, type Update } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";
import { open } from "@tauri-apps/plugin-dialog";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { openUrl } from "@tauri-apps/plugin-opener";

type Settings = {
  ramMb: number;
  javaPath: string;
  gameDir: string;
  selectedVersion: string;
  loader: string;
};

type AppInfo = { name: string; version: string };
type Page = "home" | "mods" | "emotes" | "settings" | "about";
type Toast = { id: number; kind: "ok" | "error"; text: string };
type Updater = <K extends keyof Settings>(key: K, value: Settings[K]) => void;
type JavaResult = { ok: boolean; text: string };
type AccountInfo = { signedIn: boolean; name: string; uuid: string; expiresAt: number };
type DeviceCode = { userCode: string; verificationUri: string };
type Emote = {
  id: string;
  name: string;
  author: string;
  description: string;
  bones: string[];
  sizeKb: number;
  sourceFile: string;
  icon: string | null;
};
type ModEntry = { fileName: string; enabled: boolean; name: string; modVersion: string; sizeKb: number };
type Progress = { stage: string; done: number; total: number; message: string };
type Activity = "idle" | "installing" | "launching";

const DEFAULT_SETTINGS: Settings = {
  ramMb: 4096,
  javaPath: "",
  gameDir: "",
  selectedVersion: "",
  loader: "vanilla",
};

const RAM_MIN = 2048;
const RAM_MAX = 16384;
const RAM_STEP = 512;
const RAM_PRESETS = [4096, 6144, 8192];

const TABS: { page: Page; label: string }[] = [
  { page: "home", label: "Home" },
  { page: "mods", label: "Mods" },
  { page: "emotes", label: "Emotes" },
  { page: "settings", label: "Settings" },
  { page: "about", label: "About" },
];

export default function App() {
  const [page, setPage] = useState<Page>("home");
  const [saved, setSaved] = useState<Settings | null>(null);
  const [draft, setDraft] = useState<Settings | null>(null);
  const [info, setInfo] = useState<AppInfo | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [javaResult, setJavaResult] = useState<JavaResult | null>(null);
  const [busy, setBusy] = useState(false);
  const [toasts, setToasts] = useState<Toast[]>([]);
  const nextToastId = useRef(1);

  const [versions, setVersions] = useState<string[]>([]);
  const [versionsError, setVersionsError] = useState<string | null>(null);
  const [installed, setInstalled] = useState(false);
  const [progress, setProgress] = useState<Progress | null>(null);
  const [activity, setActivity] = useState<Activity>("idle");
  const [fabricInstalled, setFabricInstalled] = useState(false);
  const [mods, setMods] = useState<ModEntry[]>([]);
  const [emotes, setEmotes] = useState<Emote[]>([]);
  const [account, setAccount] = useState<AccountInfo | null>(null);
  const [deviceCode, setDeviceCode] = useState<DeviceCode | null>(null);
  const [wheelOpen, setWheelOpen] = useState(false);

  const [updateNote, setUpdateNote] = useState("");
  const [updating, setUpdating] = useState(false);
  const gameRunningRef = useRef(false);
  const pendingUpdateRef = useRef<Update | null>(null);

  function notify(kind: Toast["kind"], text: string) {
    const id = nextToastId.current++;
    setToasts((list) => [...list, { id, kind, text }]);
    window.setTimeout(() => {
      setToasts((list) => list.filter((t) => t.id !== id));
    }, 5000);
  }

  async function applyUpdate(found: Update) {
    setUpdating(true);
    setUpdateNote(`Installing version ${found.version}. The launcher will restart.`);
    try {
      await found.downloadAndInstall();
      await relaunch();
    } catch (e) {
      setUpdating(false);
      setUpdateNote("");
      notify("error", `Update failed: ${e}`);
    }
  }

  async function findUpdate(manual: boolean) {
    try {
      const found = await check();
      if (!found) {
        if (manual) notify("ok", "You have the latest version.");
        return;
      }
      if (gameRunningRef.current) {
        pendingUpdateRef.current = found;
        setUpdateNote(`Version ${found.version} will install when Minecraft closes.`);
        return;
      }
      await applyUpdate(found);
    } catch (e) {
      if (manual) notify("error", `Could not check for updates: ${e}`);
    }
  }

  useEffect(() => {
    invoke<Settings>("load_settings")
      .then((s) => {
        setSaved(s);
        setDraft(s);
      })
      .catch((e) => setLoadError(String(e)));

    invoke<AppInfo>("app_info")
      .then(setInfo)
      .catch(() => setInfo(null));

    invoke<string[]>("list_versions")
      .then(setVersions)
      .catch((e) => setVersionsError(String(e)));

    let cancelled = false;
    const unlisteners: Array<() => void> = [];
    const keep = (unlisten: () => void) => {
      if (cancelled) unlisten();
      else unlisteners.push(unlisten);
    };

    listen<Progress>("install-progress", (e) => setProgress(e.payload)).then(keep);
    getCurrentWebview()
      .onDragDropEvent((event) => {
        if (event.payload.type === "drop") void importEmotePaths(event.payload.paths);
      })
      .then(keep);
    invoke<Emote[]>("list_emotes").then(setEmotes).catch(() => setEmotes([]));
    void refreshAccount();
    listen("auth-changed", () => {
      setDeviceCode(null);
      void refreshAccount();
      notify("ok", "Signed in to Microsoft");
    }).then(keep);
    listen<string>("auth-failed", (e) => {
      setDeviceCode(null);
      notify("error", e.payload);
    }).then(keep);
    listen<number | null>("game-exited", () => {
      gameRunningRef.current = false;
      notify("ok", "Minecraft has closed.");
      const waiting = pendingUpdateRef.current;
      if (waiting) {
        pendingUpdateRef.current = null;
        void applyUpdate(waiting);
      }
    }).then(keep);

    const startupCheck = window.setTimeout(() => void findUpdate(false), 4000);

    return () => {
      cancelled = true;
      window.clearTimeout(startupCheck);
      unlisteners.forEach((u) => u());
    };
  }, []);

  useEffect(() => {
    function onKey(e: KeyboardEvent) {
      if (e.ctrlKey || e.metaKey || e.altKey) return;
      if (e.key === "Escape") setWheelOpen(false);
      const target = e.target as HTMLElement | null;
      const typing =
        !!target &&
        (target.tagName === "INPUT" ||
          target.tagName === "TEXTAREA" ||
          target.tagName === "SELECT" ||
          target.isContentEditable);
      if (!typing && (e.key === "b" || e.key === "B")) {
        e.preventDefault();
        setWheelOpen((open) => !open);
      }
    }
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  const dirty = useMemo(
    () => saved !== null && draft !== null && JSON.stringify(saved) !== JSON.stringify(draft),
    [saved, draft],
  );

  const selectedVersion = draft?.selectedVersion ?? "";

  useEffect(() => {
    if (!selectedVersion) {
      setInstalled(false);
      setFabricInstalled(false);
      setMods([]);
      return;
    }
    invoke<boolean>("is_installed", { version: selectedVersion })
      .then(setInstalled)
      .catch(() => setInstalled(false));
    invoke<boolean>("is_fabric_installed", { version: selectedVersion })
      .then(setFabricInstalled)
      .catch(() => setFabricInstalled(false));
    void refreshMods(selectedVersion);
  }, [selectedVersion]);

  const update: Updater = (key, value) => {
    setDraft((d) => (d ? { ...d, [key]: value } : d));
  };

  async function save(): Promise<boolean> {
    if (!draft) return false;
    setBusy(true);
    try {
      await invoke("save_settings", { settings: draft });
      setSaved(draft);
      notify("ok", "Settings saved");
      return true;
    } catch (e) {
      notify("error", `Save failed: ${e}`);
      return false;
    } finally {
      setBusy(false);
    }
  }

  function discard() {
    if (saved) setDraft(saved);
  }

  async function checkJava() {
    if (!draft) return;
    setBusy(true);
    setJavaResult(null);
    try {
      const line = await invoke<string>("check_java", { path: draft.javaPath });
      setJavaResult({ ok: true, text: line });
    } catch (e) {
      setJavaResult({ ok: false, text: String(e) });
    } finally {
      setBusy(false);
    }
  }

  function selectVersion(version: string) {
    update("selectedVersion", version);
  }

  async function install() {
    if (!selectedVersion) return;
    if (draft?.loader === "fabric") {
      await installFabric();
      return;
    }
    setActivity("installing");
    setProgress(null);
    try {
      const message = await invoke<string>("install_version", { version: selectedVersion });
      setInstalled(true);
      notify("ok", message);
    } catch (e) {
      notify("error", String(e));
    } finally {
      setActivity("idle");
    }
  }

  async function play() {
    if (!selectedVersion) return;
    // Launching reads the saved settings, so save pending changes first.
    if (dirty) {
      const ok = await save();
      if (!ok) return;
    }
    setActivity("launching");
    setProgress(null);
    try {
      const message = await invoke<string>("launch_game", { version: selectedVersion });
      gameRunningRef.current = true;
      setInstalled(true);
      setFabricInstalled(true);
      notify("ok", message);
    } catch (e) {
      notify("error", String(e));
    } finally {
      setActivity("idle");
    }
  }

  async function refreshMods(version: string) {
    if (!version) {
      setMods([]);
      return;
    }
    try {
      setMods(await invoke<ModEntry[]>("list_mods", { version }));
    } catch {
      setMods([]);
    }
  }

  async function installFabric() {
    if (!selectedVersion) return;
    setActivity("installing");
    setProgress(null);
    try {
      const message = await invoke<string>("install_fabric", { version: selectedVersion });
      setInstalled(true);
      setFabricInstalled(true);
      notify("ok", message);
      await refreshMods(selectedVersion);
    } catch (e) {
      notify("error", String(e));
    } finally {
      setActivity("idle");
    }
  }

  async function addMods() {
    if (!selectedVersion) return;
    try {
      const picked = await open({
        multiple: true,
        directory: false,
        filters: [{ name: "Fabric mods", extensions: ["jar"] }],
      });
      if (!picked) return;
      const paths = Array.isArray(picked) ? picked : [picked];
      const count = await invoke<number>("add_mods", { version: selectedVersion, paths });
      notify("ok", `Added ${count} mod${count === 1 ? "" : "s"}`);
      await refreshMods(selectedVersion);
    } catch (e) {
      notify("error", String(e));
    }
  }

  async function toggleMod(mod: ModEntry) {
    try {
      await invoke("set_mod_enabled", {
        version: selectedVersion,
        fileName: mod.fileName,
        enabled: !mod.enabled,
      });
      await refreshMods(selectedVersion);
    } catch (e) {
      notify("error", String(e));
    }
  }

  async function removeMod(mod: ModEntry) {
    if (!window.confirm(`Remove ${mod.name || mod.fileName} from this instance?`)) return;
    try {
      await invoke("remove_mod", { version: selectedVersion, fileName: mod.fileName });
      notify("ok", "Mod removed");
      await refreshMods(selectedVersion);
    } catch (e) {
      notify("error", String(e));
    }
  }

  async function importEmotePaths(paths: string[]) {
    const valid = paths.filter((p) => p.toLowerCase().endsWith(".emotecraft"));
    if (valid.length === 0) {
      notify("error", "Only .emotecraft files can be added to the emote library.");
      return;
    }
    try {
      const list = await invoke<Emote[]>("import_emotes", { paths: valid });
      setEmotes(list);
      notify("ok", `Imported ${valid.length} emote${valid.length === 1 ? "" : "s"}`);
      setPage("emotes");
    } catch (e) {
      notify("error", String(e));
    }
  }

  async function pickEmoteFiles() {
    try {
      const picked = await open({
        multiple: true,
        filters: [{ name: "Emotecraft emotes", extensions: ["emotecraft"] }],
      });
      if (!picked) return;
      await importEmotePaths(Array.isArray(picked) ? picked : [picked]);
    } catch (e) {
      notify("error", String(e));
    }
  }

  async function removeEmote(emote: Emote) {
    if (!window.confirm(`Remove ${emote.name} from the library?`)) return;
    try {
      await invoke("remove_emote", { id: emote.id });
      setEmotes((list) => list.filter((x) => x.id !== emote.id));
      notify("ok", "Emote removed");
    } catch (e) {
      notify("error", String(e));
    }
  }

  async function refreshAccount() {
    try {
      setAccount(await invoke<AccountInfo>("account_status"));
    } catch (e) {
      notify("error", `Could not check your Microsoft sign-in: ${e}`);
    }
  }

  async function startSignIn() {
    try {
      setDeviceCode(await invoke<DeviceCode>("sign_in_start"));
    } catch (e) {
      notify("error", String(e));
    }
  }

  async function signOut() {
    try {
      await invoke("sign_out");
      setAccount(null);
      notify("ok", "Signed out");
    } catch (e) {
      notify("error", String(e));
    }
  }

  if (loadError) {
    return (
      <Splash error>
        <p>Could not read your settings.</p>
        <p className="splash-detail">{loadError}</p>
        <button
          className="btn primary"
          onClick={() => {
            setSaved(DEFAULT_SETTINGS);
            setDraft(DEFAULT_SETTINGS);
            setLoadError(null);
          }}
        >
          Continue with defaults
        </button>
      </Splash>
    );
  }

  if (!draft) {
    return <Splash>Loading</Splash>;
  }

  return (
    <div className="app">
      <div className="bg" aria-hidden="true">
        <span className="orb orb-a" />
        <span className="orb orb-b" />
        <span className="orb orb-c" />
        <span className="grid-floor" />
      </div>

      <header className="topbar">
        <div className="brand">
          <img src="/logo.png" alt="" />
          <div>
            <div className="brand-name">AURORA</div>
            <div className="brand-sub">CLIENT</div>
          </div>
        </div>

        <nav className="tabs" aria-label="Main">
          {TABS.map((t) => (
            <button
              key={t.page}
              className={`tab ${page === t.page ? "active" : ""}`}
              onClick={() => setPage(t.page)}
              aria-current={page === t.page ? "page" : undefined}
            >
              {t.label}
            </button>
          ))}
          <span className="tab soon" title="Coming in a later milestone">
            Cosmetics <em>soon</em>
          </span>
          <span className="tab soon" title="Coming in a later milestone">
            Marketplace <em>soon</em>
          </span>
        </nav>

        <div className="top-right">
          <span className={`chip ${dirty ? "warn" : ""}`}>{dirty ? "Unsaved changes" : "All changes saved"}</span>
          <span className="version-pill">v{info?.version ?? "0.0.0"}</span>
        </div>
      </header>

      {updateNote && <div className="update-banner">{updateNote}</div>}

      <main key={page} className="page">
        {page === "home" && (
          <HomePage
            draft={draft}
            versions={versions}
            versionsError={versionsError}
            installed={draft.loader === "fabric" ? fabricInstalled : installed}
            activity={activity}
            progress={progress}
            onSelectVersion={selectVersion}
            onSelectLoader={(l) => update("loader", l)}
            onInstall={install}
            onPlay={play}
            onOpen={setPage}
            account={account}
            onSignIn={() => void startSignIn()}
            onSignOut={() => void signOut()}
          />
        )}
        {page === "mods" && (
          <ModsPage
            version={selectedVersion}
            loader={draft.loader}
            fabricInstalled={fabricInstalled}
            mods={mods}
            busy={activity !== "idle"}
            progress={progress}
            onInstallFabric={() => void installFabric()}
            onAdd={() => void addMods()}
            onToggle={(m) => void toggleMod(m)}
            onRemove={(m) => void removeMod(m)}
            onOpenHome={() => setPage("home")}
          />
        )}
        {page === "emotes" && (
          <EmotesPage
            emotes={emotes}
            onPick={() => void pickEmoteFiles()}
            onRemove={(e) => void removeEmote(e)}
            onOpenWheel={() => setWheelOpen(true)}
          />
        )}
        {page === "settings" && (
          <SettingsPage
            draft={draft}
            dirty={dirty}
            busy={busy}
            javaResult={javaResult}
            update={update}
            onSave={save}
            onDiscard={discard}
            onCheckJava={checkJava}
          />
        )}
        {page === "about" && (
          <AboutPage info={info} updating={updating} onCheckUpdates={() => void findUpdate(true)} />
        )}
      </main>

      {deviceCode && <SignInModal prompt={deviceCode} onClose={() => setDeviceCode(null)} />}

      {wheelOpen && (
        <EmoteWheel
          emotes={emotes}
          onClose={() => setWheelOpen(false)}
          onPick={(e) => {
            setWheelOpen(false);
            notify("ok", `Selected ${e.name}. Playback needs the in-game client.`);
          }}
        />
      )}

      <div className="toasts" role="status" aria-live="polite">
        {toasts.map((t) => (
          <div key={t.id} className={`toast ${t.kind}`}>
            {t.text}
          </div>
        ))}
      </div>
    </div>
  );
}

function Splash({ children, error = false }: { children: ReactNode; error?: boolean }) {
  return (
    <div className="splash">
      <div className="aurora" aria-hidden="true">
        <span className="band band-a" />
        <span className="band band-b" />
        <span className="band band-c" />
      </div>
      <div className="stars" aria-hidden="true" />
      <div className="splash-content">
        <img className="splash-logo" src="/logo.png" alt="Aurora" />
        <div className="aurora-title">AURORA</div>
        <div className={`splash-text ${error ? "error" : ""}`}>{children}</div>
      </div>
    </div>
  );
}

/* Embers drawn on a canvas. Holds still when the system asks for reduced motion. */
function Embers() {
  const ref = useRef<HTMLCanvasElement>(null);

  useEffect(() => {
    const canvas = ref.current;
    if (!canvas) return;
    const ctx = canvas.getContext("2d");
    if (!ctx) return;

    type Ember = { x: number; y: number; r: number; vy: number; vx: number; a: number; hue: number };
    const reduceMotion = window.matchMedia("(prefers-reduced-motion: reduce)").matches;
    let w = 0;
    let h = 0;
    let embers: Ember[] = [];
    let frameId = 0;

    const spawn = (anywhere: boolean): Ember => ({
      x: Math.random() * w,
      y: anywhere ? Math.random() * h : h + 10,
      r: 0.6 + Math.random() * 1.8,
      vy: 0.25 + Math.random() * 0.7,
      vx: (Math.random() - 0.5) * 0.25,
      a: 0.3 + Math.random() * 0.7,
      hue: 18 + Math.random() * 32,
    });

    const draw = () => {
      ctx.clearRect(0, 0, w, h);
      for (const e of embers) {
        ctx.beginPath();
        ctx.fillStyle = `hsla(${e.hue}, 100%, 60%, ${e.a})`;
        ctx.arc(e.x, e.y, e.r, 0, Math.PI * 2);
        ctx.fill();
      }
    };

    const step = () => {
      for (const e of embers) {
        e.y -= e.vy;
        e.x += e.vx + Math.sin(e.y / 40) * 0.15;
        if (e.y < -10) Object.assign(e, spawn(false));
      }
      draw();
      frameId = requestAnimationFrame(step);
    };

    const resize = () => {
      const dpr = window.devicePixelRatio || 1;
      w = canvas.clientWidth;
      h = canvas.clientHeight;
      canvas.width = Math.max(1, Math.floor(w * dpr));
      canvas.height = Math.max(1, Math.floor(h * dpr));
      ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
      const count = Math.min(160, Math.round((w * h) / 9000));
      embers = Array.from({ length: count }, () => spawn(true));
      draw();
    };

    resize();
    window.addEventListener("resize", resize);
    if (!reduceMotion) frameId = requestAnimationFrame(step);

    return () => {
      cancelAnimationFrame(frameId);
      window.removeEventListener("resize", resize);
    };
  }, []);

  return <canvas ref={ref} className="embers" aria-hidden="true" />;
}

type HomeProps = {
  draft: Settings;
  versions: string[];
  versionsError: string | null;
  installed: boolean;
  activity: Activity;
  progress: Progress | null;
  onSelectVersion: (version: string) => void;
  onSelectLoader: (loader: string) => void;
  onInstall: () => void;
  onPlay: () => void;
  onOpen: (page: Page) => void;
  account: AccountInfo | null;
  onSignIn: () => void;
  onSignOut: () => void;
};

function HomePage({
  draft,
  versions,
  versionsError,
  installed,
  activity,
  progress,
  onSelectVersion,
  onSelectLoader,
  onInstall,
  onPlay,
  onOpen,
  account,
  onSignIn,
  onSignOut,
}: HomeProps) {
  const selected = draft.selectedVersion;
  const busy = activity !== "idle";
  const javaName = draft.javaPath.trim() ? draft.javaPath.trim().split(/[\\/]/).pop() : "java from PATH";
  const ramGb = (draft.ramMb / 1024).toFixed(1);

  return (
    <div className="home">
      <section className="hero">
        <Embers />
        <div className="hero-core" aria-hidden="true">
          <div className="ring ring-outer" />
          <div className="ring ring-inner" />
          <img className="hero-logo" src="/logo.png" alt="" />
        </div>

        <div className="hero-text">
          <span className="eyebrow">Unofficial Minecraft Java launcher</span>
          <h1>Aurora</h1>
          <p>Launch, manage, and customise Minecraft from one place.</p>
        </div>
      </section>

      <section className="dock">
        <div className="card account">
          <div className="avatar" aria-hidden="true">
            {account?.signedIn ? account.name.charAt(0).toUpperCase() : "?"}
          </div>
          <div className="account-info">
            <div className="account-name">{account?.signedIn ? account.name : "Not signed in"}</div>
            <div className="muted small">
              {account?.signedIn
                ? "Signed in. The session refreshes automatically."
                : "Sign in with a Microsoft account that owns Minecraft"}
            </div>
          </div>
          {account?.signedIn ? (
            <button className="btn" onClick={onSignOut}>
              Sign out
            </button>
          ) : (
            <button className="btn primary" onClick={onSignIn}>
              Sign in
            </button>
          )}
        </div>

        <div className="card play-block">
          <button className="play-btn" onClick={onPlay} disabled={busy || !selected}>
            {activity === "launching" ? "STARTING..." : "PLAY"}
          </button>
          <div className="segmented" role="group" aria-label="Loader">
            {["vanilla", "fabric"].map((l) => (
              <button
                key={l}
                className={`seg ${draft.loader === l ? "active" : ""}`}
                onClick={() => onSelectLoader(l)}
                disabled={busy}
              >
                {l === "vanilla" ? "Vanilla" : "Fabric"}
              </button>
            ))}
          </div>
          <div className="version-row">
            <select
              className="select"
              value={selected}
              onChange={(e) => onSelectVersion(e.target.value)}
              disabled={busy || versions.length === 0}
              aria-label="Minecraft version"
            >
              <option value="">{versions.length ? "Choose a version" : "Loading versions..."}</option>
              {versions.map((v) => (
                <option key={v} value={v}>
                  {v}
                </option>
              ))}
            </select>
            <button className="btn" onClick={onInstall} disabled={busy || !selected || installed}>
              {installed ? "Installed" : "Install"}
            </button>
          </div>
          <small className="muted">
            {versionsError
              ? `Could not load versions: ${versionsError}`
              : "Playing needs a signed-in Microsoft account that owns Minecraft Java Edition."}
          </small>
          {busy && progress && <ProgressBar progress={progress} />}
        </div>

        <div className="card stats">
          <button className="stat" onClick={() => onOpen("settings")}>
            <span className="stat-label">Memory</span>
            <span className="stat-value">{ramGb} GB</span>
          </button>
          <button className="stat" onClick={() => onOpen("settings")}>
            <span className="stat-label">Java</span>
            <span className="stat-value">{javaName ?? "java"}</span>
          </button>
          <div className="stat">
            <span className="stat-label">Minecraft</span>
            <span className="stat-value">{selected || "None selected"}</span>
            <span className="stat-hint">
              {installed ? "Installed and verified" : selected ? "Not installed yet" : "Pick a version"}
            </span>
          </div>
        </div>
      </section>

      <section className="features">
        <FeatureCard art="art-cosmetics" title="Cosmetics" text="Capes and skins, previewed in game." />
        <FeatureCard art="art-emotes" title="Emotes" text="Animated emotes with custom keybinds." />
        <FeatureCard art="art-market" title="Marketplace" text="Browse and buy items with points." />
        <FeatureCard art="art-rewards" title="Daily rewards" text="Log in each day to earn points." />
      </section>

      <section className="card roadmap-card">
        <h3>Roadmap</h3>
        <ul className="roadmap">
          <RoadItem state="done" text="Launcher interface, settings, and Java check" />
          <RoadItem state="done" text="Version list, verified downloads, and native extraction" />
          <RoadItem state="done" text="Signed automatic updates" />
          <RoadItem state="done" text="Microsoft sign-in (needs an approved Azure client ID)" />
          <RoadItem state="todo" text="Backend, accounts, and permissions" />
          <RoadItem state="todo" text="In-game client, cosmetics, emotes, and marketplace" />
        </ul>
      </section>
    </div>
  );
}

function FeatureCard({ art, title, text }: { art: string; title: string; text: string }) {
  return (
    <div className="card feature">
      <div className={`feature-art ${art}`} aria-hidden="true" />
      <div className="feature-body">
        <div className="feature-title">
          {title}
          <span className="badge">Soon</span>
        </div>
        <p className="muted small">{text}</p>
      </div>
    </div>
  );
}

function ProgressBar({ progress }: { progress: Progress }) {
  const indeterminate = progress.total === 0;
  const pct = indeterminate ? 0 : Math.round((progress.done / progress.total) * 100);
  return (
    <div className={`progress ${indeterminate ? "indeterminate" : ""}`}>
      <div className="progress-head">
        <span>{progress.stage}</span>
        <span>{indeterminate ? "" : `${progress.done} / ${progress.total} (${pct}%)`}</span>
      </div>
      <div className="progress-track">
        <div className="progress-fill" style={{ width: indeterminate ? "35%" : `${pct}%` }} />
      </div>
      {progress.message && <small className="muted">{progress.message}</small>}
    </div>
  );
}

function RoadItem({ state, text }: { state: "done" | "next" | "todo"; text: string }) {
  const label = state === "done" ? "Done" : state === "next" ? "Next" : "Planned";
  return (
    <li className={state}>
      <span className={`state ${state}`}>{label}</span>
      <span>{text}</span>
    </li>
  );
}

type SettingsProps = {
  draft: Settings;
  dirty: boolean;
  busy: boolean;
  javaResult: JavaResult | null;
  update: Updater;
  onSave: () => Promise<boolean>;
  onDiscard: () => void;
  onCheckJava: () => void;
};

function SettingsPage({ draft, dirty, busy, javaResult, update, onSave, onDiscard, onCheckJava }: SettingsProps) {
  return (
    <div className="narrow">
      <section className="card">
        <div className="panel-head">
          <h3>Memory</h3>
          <span className="value-pill">{draft.ramMb} MB</span>
        </div>
        <input
          className="slider"
          type="range"
          min={RAM_MIN}
          max={RAM_MAX}
          step={RAM_STEP}
          value={draft.ramMb}
          onChange={(e) => update("ramMb", Number(e.target.value))}
          aria-label="RAM allocated to Minecraft"
        />
        <div className="presets">
          {RAM_PRESETS.map((mb) => (
            <button
              key={mb}
              className={`preset ${draft.ramMb === mb ? "active" : ""}`}
              onClick={() => update("ramMb", mb)}
            >
              {mb / 1024} GB
            </button>
          ))}
        </div>
        <p className="hint">Minecraft 1.21 runs comfortably with 4 GB or more.</p>
      </section>

      <section className="card">
        <h3>Java</h3>
        <label className="field">
          <span>Java executable</span>
          <input
            type="text"
            value={draft.javaPath}
            placeholder="Leave empty to use java from PATH"
            onChange={(e) => update("javaPath", e.target.value)}
          />
        </label>
        <p className="hint">Minecraft 1.21 needs Java 21. Example: C:\Program Files\Java\jdk-21\bin\java.exe</p>
        <div className="row">
          <button className="btn" onClick={onCheckJava} disabled={busy}>
            {busy ? "Checking..." : "Check Java"}
          </button>
        </div>
        {javaResult && <div className={`terminal ${javaResult.ok ? "ok" : "err"}`}>{javaResult.text}</div>}
      </section>

      <section className="card">
        <h3>Game folder</h3>
        <label className="field">
          <span>Folder for Minecraft files</span>
          <input
            type="text"
            value={draft.gameDir}
            placeholder="Leave empty for the default folder"
            onChange={(e) => update("gameDir", e.target.value)}
          />
        </label>
        <p className="hint">Versions, libraries, assets, and logs are stored here. Leave empty to use AppData.</p>
      </section>

      {dirty && (
        <div className="savebar">
          <span>You have unsaved changes</span>
          <div className="row">
            <button className="btn ghost" onClick={onDiscard} disabled={busy}>
              Discard
            </button>
            <button className="btn primary" onClick={() => void onSave()} disabled={busy}>
              Save changes
            </button>
          </div>
        </div>
      )}
    </div>
  );
}

function AboutPage({
  info,
  updating,
  onCheckUpdates,
}: {
  info: AppInfo | null;
  updating: boolean;
  onCheckUpdates: () => void;
}) {
  return (
    <div className="narrow">
      <section className="card about-hero">
        <img src="/logo.png" alt="" />
        <div>
          <h3>{info?.name ?? "Aurora"}</h3>
          <p className="muted">Version {info?.version ?? "unknown"}</p>
          <p className="muted small">
            An unofficial launcher project for Minecraft Java Edition. It is not affiliated with or endorsed by
            Mojang Studios or Microsoft. Minecraft is a trademark of Mojang Studios.
          </p>
        </div>
      </section>

      <section className="card">
        <div className="panel-head">
          <h3>Updates</h3>
          <button className="btn" onClick={onCheckUpdates} disabled={updating}>
            {updating ? "Updating..." : "Check for updates"}
          </button>
        </div>
        <p className="muted">
          The launcher checks for updates a few seconds after it starts and installs them automatically. If
          Minecraft is running, the update waits until the game closes. Updates are signed, and the launcher
          rejects any update whose signature doesn't match.
        </p>
      </section>

      <section className="card">
        <h3>Your data</h3>
        <p className="muted">
          Settings are stored in your Windows user AppData folder (com.aurora.launcher). Updates do not
          touch that folder. Minecraft files are stored in the Game folder, or in AppData if none is set.
        </p>
      </section>
    </div>
  );
}

type ModsPageProps = {
  version: string;
  loader: string;
  fabricInstalled: boolean;
  mods: ModEntry[];
  busy: boolean;
  progress: Progress | null;
  onInstallFabric: () => void;
  onAdd: () => void;
  onToggle: (mod: ModEntry) => void;
  onRemove: (mod: ModEntry) => void;
  onOpenHome: () => void;
};

function ModsPage({
  version,
  loader,
  fabricInstalled,
  mods,
  busy,
  progress,
  onInstallFabric,
  onAdd,
  onToggle,
  onRemove,
  onOpenHome,
}: ModsPageProps) {
  if (!version) {
    return (
      <div className="narrow">
        <section className="card">
          <h3>Choose a version first</h3>
          <p className="muted">Pick a Minecraft version on Home. Mods are kept separately for each version.</p>
          <div className="row">
            <button className="btn" onClick={onOpenHome}>
              Go to Home
            </button>
          </div>
        </section>
      </div>
    );
  }

  if (!fabricInstalled) {
    return (
      <div className="narrow">
        <section className="card">
          <h3>Fabric is not installed for {version}</h3>
          <p className="muted">
            Mods need the Fabric loader. Installing it also downloads the game files for {version}, a one-time
            download.
          </p>
          <div className="row">
            <button className="btn primary" onClick={onInstallFabric} disabled={busy}>
              Install Fabric
            </button>
          </div>
          {busy && progress && <ProgressBar progress={progress} />}
        </section>
      </div>
    );
  }

  return (
    <div className="narrow">
      <section className="card">
        <div className="panel-head">
          <div>
            <h3>Mods for {version}</h3>
            <small className="muted">Mods load only when the loader is set to Fabric. Many mods also need Fabric API.</small>
          </div>
          <button className="btn primary" onClick={onAdd} disabled={busy}>
            Add mods
          </button>
        </div>

        {loader !== "fabric" && (
          <p className="hint">The loader is set to Vanilla, so these mods will not load. Switch to Fabric on Home.</p>
        )}

        {mods.length === 0 ? (
          <p className="muted">No mods yet. Click Add mods and choose Fabric .jar files.</p>
        ) : (
          <ul className="mod-list">
            {mods.map((m) => (
              <li key={m.fileName} className={`mod-row ${m.enabled ? "" : "off"}`}>
                <div className="mod-info">
                  <div className="mod-name">{m.name || m.fileName}</div>
                  <small className="muted">
                    {[m.modVersion, `${m.sizeKb} KB`, m.fileName].filter(Boolean).join(" · ")}
                  </small>
                </div>
                <button
                  className={`switch ${m.enabled ? "on" : ""}`}
                  role="switch"
                  aria-checked={m.enabled}
                  aria-label={`Enable ${m.name || m.fileName}`}
                  onClick={() => onToggle(m)}
                  disabled={busy}
                >
                  <span />
                </button>
                <button className="btn ghost small-btn" onClick={() => onRemove(m)} disabled={busy}>
                  Remove
                </button>
              </li>
            ))}
          </ul>
        )}

        {busy && progress && <ProgressBar progress={progress} />}
      </section>
    </div>
  );
}

type EmotesPageProps = {
  emotes: Emote[];
  onPick: () => void;
  onRemove: (emote: Emote) => void;
  onOpenWheel: () => void;
};

function EmotesPage({ emotes, onPick, onRemove, onOpenWheel }: EmotesPageProps) {
  return (
    <div className="narrow">
      <section className="card dropzone">
        <div>
          <h3>Emote library</h3>
          <p className="muted">
            Drop .emotecraft files anywhere in this window, or choose them here. The launcher copies them into
            its library and leaves the original files alone.
          </p>
        </div>
        <div className="row">
          <button className="btn primary" onClick={onPick}>
            Choose files
          </button>
          <button className="btn" onClick={onOpenWheel} disabled={emotes.length === 0}>
            Open emote wheel (B)
          </button>
        </div>
      </section>

      {emotes.length === 0 ? (
        <p className="muted">No emotes yet.</p>
      ) : (
        <div className="emote-grid">
          {emotes.map((e) => (
            <div key={e.id} className="card emote-card">
              <div className="emote-icon">{e.icon ? <img src={e.icon} alt="" /> : <span>?</span>}</div>
              <div className="emote-name">{e.name}</div>
              <small className="muted">{e.author ? `by ${e.author}` : "Author not listed"}</small>
              {e.description && <p className="muted small">{e.description}</p>}
              <div className="chips">
                {e.bones.map((b) => (
                  <span key={b} className="chip-bone">
                    {b}
                  </span>
                ))}
              </div>
              <div className="emote-foot">
                <small className="muted">{e.sizeKb} KB</small>
                <button className="btn ghost small-btn" onClick={() => onRemove(e)}>
                  Remove
                </button>
              </div>
            </div>
          ))}
        </div>
      )}

      <p className="hint">
        Press B anywhere in the launcher to open the emote wheel. Emotes play in the game once the in-game client
        is built. Only add emotes whose creators allow you to use them.
      </p>
    </div>
  );
}

function shorten(text: string, max: number): string {
  return text.length > max ? `${text.slice(0, max - 1)}…` : text;
}

type WheelProps = {
  emotes: Emote[];
  onClose: () => void;
  onPick: (emote: Emote) => void;
};

function EmoteWheel({ emotes, onClose, onPick }: WheelProps) {
  const [hover, setHover] = useState<number | null>(null);
  const size = 440;
  const c = size / 2;
  const rOuter = 200;
  const rInner = 78;
  const n = emotes.length;
  const point = (angle: number, r: number) =>
    `${(c + r * Math.cos(angle)).toFixed(2)} ${(c + r * Math.sin(angle)).toFixed(2)}`;
  const active = hover !== null ? emotes[hover] : null;

  return (
    <div className="wheel-backdrop" onClick={onClose}>
      <div className="wheel" role="dialog" aria-label="Emote wheel" onClick={(e) => e.stopPropagation()}>
        {n === 0 ? (
          <p className="muted">No emotes yet. Drop .emotecraft files into the launcher first.</p>
        ) : (
          <svg viewBox={`0 0 ${size} ${size}`} width={size} height={size}>
            {n === 1 ? (
              <circle
                cx={c}
                cy={c}
                r={(rOuter + rInner) / 2}
                strokeWidth={rOuter - rInner}
                fill="none"
                className={`slice ${hover === 0 ? "hot" : ""}`}
                onMouseEnter={() => setHover(0)}
                onMouseLeave={() => setHover(null)}
                onClick={() => onPick(emotes[0])}
              />
            ) : (
              emotes.map((e, i) => {
                const a0 = (i / n) * Math.PI * 2 - Math.PI / 2;
                const a1 = ((i + 1) / n) * Math.PI * 2 - Math.PI / 2;
                const large = a1 - a0 > Math.PI ? 1 : 0;
                const d =
                  `M ${point(a0, rOuter)} A ${rOuter} ${rOuter} 0 ${large} 1 ${point(a1, rOuter)} ` +
                  `L ${point(a1, rInner)} A ${rInner} ${rInner} 0 ${large} 0 ${point(a0, rInner)} Z`;
                const mid = (a0 + a1) / 2;
                const rMid = (rOuter + rInner) / 2;
                return (
                  <g
                    key={e.id}
                    className="slice-group"
                    onMouseEnter={() => setHover(i)}
                    onMouseLeave={() => setHover(null)}
                    onClick={() => onPick(e)}
                  >
                    <path d={d} className={`slice ${hover === i ? "hot" : ""}`} />
                    <text
                      x={c + rMid * Math.cos(mid)}
                      y={c + rMid * Math.sin(mid)}
                      className="slice-label"
                      textAnchor="middle"
                      dominantBaseline="middle"
                    >
                      {shorten(e.name, 14)}
                    </text>
                  </g>
                );
              })
            )}
            <circle cx={c} cy={c} r={rInner - 4} className="wheel-core" />
          </svg>
        )}

        {n > 0 && (
          <div className="wheel-center">
            {active ? (
              <>
                {active.icon && <img src={active.icon} alt="" />}
                <div className="wheel-name">{active.name}</div>
                <small className="muted">{active.author ? `by ${active.author}` : ""}</small>
              </>
            ) : (
              <small className="muted">Hover an emote</small>
            )}
          </div>
        )}
      </div>
    </div>
  );
}

function SignInModal({ prompt, onClose }: { prompt: DeviceCode; onClose: () => void }) {
  return (
    <div className="wheel-backdrop" onClick={onClose}>
      <div className="card signin" onClick={(e) => e.stopPropagation()}>
        <h3>Sign in with Microsoft</h3>
        <p className="muted">Open the Microsoft page and enter this code:</p>
        <div className="user-code">{prompt.userCode}</div>
        <div className="row">
          <button className="btn primary" onClick={() => void openUrl(prompt.verificationUri)}>
            Open Microsoft sign-in
          </button>
          <button className="btn ghost" onClick={onClose}>
            Close
          </button>
        </div>
        <small className="muted">
          Aurora signs you in automatically after you approve. Closing this window does not cancel it.
        </small>
      </div>
    </div>
  );
}
