import { PRODUCT_NAME, Wordmark } from "../components/Wordmark";
import { Page } from "./settings";

export function AboutPage() {
  return (
    <Page title="About">
      <section className="mt-6 card p-6 sm:p-7">
        <Wordmark />
        <p className="mt-3 max-w-[65ch] text-sm text-muted">
          {PRODUCT_NAME} is one self-hosted server for your films, shows and music: browse the catalog, request a title,
          let your own download client fetch it, and play it in the browser.
        </p>
        <dl className="mt-4 grid grid-cols-[8rem_1fr] gap-y-2 text-sm">
          <dt className="text-faint">License</dt>
          <dd>GPL-3.0</dd>
        </dl>
      </section>
      <section className="mt-6 card p-6 sm:p-7">
        <h2 className="text-base font-semibold">Data sources</h2>
        <ul className="mt-2 space-y-2 text-sm text-muted">
          <li>
            Film and show data and images from{" "}
            <a href="https://www.themoviedb.org" className="text-fg underline" target="_blank" rel="noreferrer">
              TMDB
            </a>
            . This product uses the TMDB API but is not endorsed or certified by TMDB.
          </li>
          <li>
            Music data from{" "}
            <a href="https://musicbrainz.org" className="text-fg underline" target="_blank" rel="noreferrer">
              MusicBrainz
            </a>
            , cover art from the Cover Art Archive.
          </li>
        </ul>
      </section>
    </Page>
  );
}
