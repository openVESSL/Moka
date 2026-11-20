import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useEffect, useState } from "react";
import { api, unwrap, type Schemas } from "../api/client";

type HwAccel = Schemas["HwAccel"];

/** How films are converted when a browser cannot play them as they are. */
export function PlaybackSection() {
  const qc = useQueryClient();
  const { data } = useQuery({
    queryKey: ["settings", "playback"],
    queryFn: () => unwrap(api.GET("/api/v1/settings/playback")),
  });
  const [f, setF] = useState<HwAccel>({ encoder: "software", device: "" });
  useEffect(() => {
    if (data) setF(data);
  }, [data]);
  const save = useMutation({
    mutationFn: () => unwrap(api.PUT("/api/v1/settings/playback", { body: f })),
    onSuccess: (v) => qc.setQueryData(["settings", "playback"], v),
  });

  return (
    <form
      onSubmit={(e) => {
        e.preventDefault();
        save.mutate();
      }}
      className="mt-6 flex flex-wrap items-end gap-x-5 gap-y-4"
    >
      <label className="block w-72">
        <span className="field-label">Convert video with</span>
        <select
          className="field bg-canvas"
          value={f.encoder}
          onChange={(e) => setF((x) => ({ ...x, encoder: e.target.value as HwAccel["encoder"] }))}
        >
          <option value="software">Processor (works everywhere)</option>
          <option value="nvenc">NVIDIA graphics (NVENC)</option>
          <option value="vaapi">Intel or AMD graphics on Linux (VAAPI)</option>
          <option value="qsv">Intel Quick Sync (QSV)</option>
        </select>
      </label>
      {f.encoder === "vaapi" && (
        <label className="block w-72">
          <span className="field-label">Device</span>
          <input
            className="field data text-base"
            value={f.device ?? ""}
            onChange={(e) => setF((x) => ({ ...x, device: e.target.value }))}
            placeholder="/dev/dri/renderD128"
          />
        </label>
      )}
      <button type="submit" className="btn-primary" disabled={save.isPending}>
        {save.isPending ? "Saving…" : "Save"}
      </button>
      <p role="status" className={`w-full text-sm ${save.error ? "text-danger" : "text-state-available"}`}>
        {save.error?.message ?? (save.isSuccess ? "Saved. Used from the next film you start." : "")}
      </p>
    </form>
  );
}
