import { useCallback, useEffect, useState } from "react";
import { AdminView } from "./AdminView";
import { ApiError, type AuthUser, getCsrfToken, getSession, login, logout } from "./api";
import { LoginView } from "./LoginView";
import { ProjectsView } from "./ProjectsView";

type AuthState =
  | { readonly status: "loading" }
  | { readonly status: "signedOut"; readonly csrfToken: string }
  | { readonly status: "signedIn"; readonly user: AuthUser; readonly csrfToken: string }
  | { readonly status: "error"; readonly message: string };

function BootScreen({ message }: { readonly message: string }) {
  return (
    <main className="grid min-h-screen place-items-center bg-pf-background px-6">
      <div className="text-center">
        <span
          aria-hidden="true"
          className="mx-auto grid size-10 place-items-center rounded-md bg-pf-accent text-[13px] font-semibold tracking-[0.08em] text-white"
        >
          PF
        </span>
        <p role="status" className="mt-4 text-[13px] text-pf-text-secondary">
          {message}
        </p>
      </div>
    </main>
  );
}

export default function App() {
  const [auth, setAuth] = useState<AuthState>({ status: "loading" });
  const [bootAttempt, setBootAttempt] = useState(0);
  const [view, setView] = useState<"projects" | "admin">("projects");

  // biome-ignore lint/correctness/useExhaustiveDependencies: bootAttempt is the explicit retry key
  useEffect(() => {
    let cancelled = false;
    async function boot() {
      try {
        const session = await getSession();
        if (!cancelled) {
          setAuth({
            status: "signedIn",
            user: session.user,
            csrfToken: session.csrfToken,
          });
        }
      } catch (error) {
        if (cancelled) {
          return;
        }
        if (error instanceof ApiError && error.status === 401) {
          try {
            const csrfToken = await getCsrfToken();
            if (!cancelled) {
              setAuth({ status: "signedOut", csrfToken });
            }
          } catch {
            if (!cancelled) {
              setAuth({
                status: "error",
                message: "ProbeForge could not reach the API. Check that the local API is running.",
              });
            }
          }
          return;
        }
        setAuth({
          status: "error",
          message: "ProbeForge could not reach the API. Check that the local API is running.",
        });
      }
    }
    void boot();
    return () => {
      cancelled = true;
    };
  }, [bootAttempt]);

  const handleLogin = async (email: string, password: string) => {
    if (auth.status !== "signedOut") {
      return;
    }
    let csrfToken = auth.csrfToken;
    if (csrfToken === "") {
      csrfToken = await getCsrfToken();
    }

    let result: Awaited<ReturnType<typeof login>>;
    try {
      result = await login(email, password, csrfToken);
    } catch (error) {
      if (
        error instanceof ApiError &&
        error.status === 403 &&
        (error.code === "csrf_invalid" || error.code === "csrf_required")
      ) {
        csrfToken = await getCsrfToken();
        result = await login(email, password, csrfToken);
      } else {
        throw error;
      }
    }

    setAuth({
      status: "signedIn",
      user: result.user,
      csrfToken: result.csrfToken,
    });
    setView("projects");
  };

  const handleSignOut = async () => {
    if (auth.status === "signedIn") {
      try {
        await logout(auth.csrfToken);
      } catch {
        // The session may already be expired or revoked; local sign-out still applies.
      }
    }
    try {
      const csrfToken = await getCsrfToken();
      setAuth({ status: "signedOut", csrfToken });
    } catch {
      setAuth({ status: "signedOut", csrfToken: "" });
    }
    setView("projects");
  };

  const handleSessionExpired = useCallback(() => {
    void getCsrfToken()
      .then((csrfToken) => {
        setAuth({ status: "signedOut", csrfToken });
      })
      .catch(() => {
        setAuth({ status: "signedOut", csrfToken: "" });
      });
  }, []);

  if (auth.status === "loading") {
    return <BootScreen message="Checking your session..." />;
  }

  if (auth.status === "error") {
    return (
      <main className="grid min-h-screen place-items-center bg-pf-background px-6">
        <div className="w-full max-w-md rounded-lg border border-pf-border bg-pf-surface p-6 text-center">
          <span
            aria-hidden="true"
            className="mx-auto grid size-9 place-items-center rounded-md bg-pf-accent text-[12px] font-semibold tracking-[0.08em] text-white"
          >
            PF
          </span>
          <h1 className="mt-4 text-[20px] font-semibold tracking-[-0.015em]">
            API connection unavailable
          </h1>
          <p role="alert" className="mt-2 text-[13px] leading-5 text-pf-text-secondary">
            {auth.message}
          </p>
          <button
            type="button"
            onClick={() => {
              setAuth({ status: "loading" });
              setBootAttempt((attempt) => attempt + 1);
            }}
            className="mt-5 h-9 rounded-md bg-pf-accent px-4 text-[12px] font-medium text-white transition-colors hover:bg-pf-accent-hover"
          >
            Retry
          </button>
        </div>
      </main>
    );
  }

  if (auth.status === "signedOut") {
    return <LoginView onLogin={handleLogin} />;
  }

  return auth.user.isAdmin && view === "admin" ? (
    <AdminView
      user={auth.user}
      csrfToken={auth.csrfToken}
      onBack={() => setView("projects")}
      onSignOut={handleSignOut}
      onSessionExpired={handleSessionExpired}
    />
  ) : (
    <ProjectsView
      user={auth.user}
      csrfToken={auth.csrfToken}
      onSignOut={handleSignOut}
      onSessionExpired={handleSessionExpired}
      onOpenAdmin={auth.user.isAdmin ? () => setView("admin") : undefined}
    />
  );
}
