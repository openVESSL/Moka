import { useQueryClient } from "@tanstack/react-query";
import { useNavigate } from "@tanstack/react-router";
import { api, unwrap } from "../api/client";
import { authStatusQuery, meQuery } from "../api/queries";
import { AuthBoard } from "./AuthBoard";

const CLAP_MS = 320;
const wait = (ms: number) => new Promise((r) => setTimeout(r, ms));

export function SetupPage() {
  const qc = useQueryClient();
  const navigate = useNavigate();
  return (
    <AuthBoard
      title="Set up this server"
      intro="Create the admin account. You will connect your download client and indexers next; nothing is shared until you do."
      submitLabel="Create admin"
      passwordAutocomplete="new-password"
      onSubmit={async (username, password) => {
        const me = await unwrap(api.POST("/api/v1/auth/setup", { body: { username, password } }));
        qc.setQueryData(meQuery.queryKey, me);
        qc.setQueryData(authStatusQuery.queryKey, { needs_setup: false });
        await wait(CLAP_MS);
        navigate({ to: "/settings" });
      }}
    />
  );
}

export function LoginPage() {
  const qc = useQueryClient();
  const navigate = useNavigate();
  return (
    <AuthBoard
      title="Sign in"
      intro="Use the account the server admin gave you."
      submitLabel="Sign in"
      passwordAutocomplete="current-password"
      onSubmit={async (username, password) => {
        try {
          const me = await unwrap(api.POST("/api/v1/auth/login", { body: { username, password } }));
          qc.setQueryData(meQuery.queryKey, me);
        } catch (e) {
          throw new Error(
            e instanceof Error && "status" in e && e.status === 401
              ? "That name and password do not match."
              : "Could not reach the server.",
          );
        }
        await wait(CLAP_MS);
        navigate({ to: "/" });
      }}
    />
  );
}
