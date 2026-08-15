const LATEST_RELEASE_API_URL =
  "https://api.github.com/repos/fromfactory/vsparallel/releases/latest";

const INSTALLER_KINDS = [
  "macos-dmg",
  "windows-installer",
  "linux-deb",
] as const;

export type InstallerKind = (typeof INSTALLER_KINDS)[number];
export type DesktopPlatform = "macos" | "windows" | "linux";

export interface ReleaseAsset {
  name: string;
  browserDownloadUrl: string;
  size: number;
}

export interface LatestRelease {
  tagName: string;
  assets: ReleaseAsset[];
}

export type InstallerAssets = Partial<Record<InstallerKind, ReleaseAsset>>;

export interface NavigatorSnapshot {
  userAgent: string;
  platform: string;
  clientHintPlatform?: string;
  maxTouchPoints: number;
}

interface DownloadSpec {
  label: string;
  ariaLabel: string;
}

const DOWNLOAD_SPECS: Record<InstallerKind, DownloadSpec> = {
  "macos-dmg": {
    label: "Download DMG",
    ariaLabel: "Download VSParallel for macOS 12.3 or later",
  },
  "windows-installer": {
    label: "Download installer",
    ariaLabel: "Download VSParallel for Windows",
  },
  "linux-deb": {
    label: "Download DEB",
    ariaLabel: "Download the VSParallel Debian package for Linux",
  },
};

const PREFERRED_DOWNLOADS: Record<DesktopPlatform, InstallerKind[]> = {
  macos: ["macos-dmg"],
  windows: ["windows-installer"],
  linux: ["linux-deb"],
};

const isRecord = (value: unknown): value is Record<string, unknown> =>
  typeof value === "object" && value !== null;

const isInstallerKind = (value: string | undefined): value is InstallerKind =>
  value !== undefined && INSTALLER_KINDS.some((kind) => kind === value);

const parseReleaseAsset = (value: unknown): ReleaseAsset | null => {
  if (!isRecord(value)) {
    return null;
  }

  const name = value.name;
  const browserDownloadUrl = value.browser_download_url;
  const size = value.size;
  const state = value.state;

  if (
    typeof name !== "string" ||
    name.trim() === "" ||
    typeof browserDownloadUrl !== "string" ||
    typeof size !== "number" ||
    !Number.isFinite(size) ||
    size <= 0 ||
    (state !== undefined && state !== "uploaded")
  ) {
    return null;
  }

  try {
    const url = new URL(browserDownloadUrl);
    const expectedPath = "/fromfactory/vsparallel/releases/download/";

    if (
      url.protocol !== "https:" ||
      url.hostname !== "github.com" ||
      !url.pathname.toLowerCase().startsWith(expectedPath)
    ) {
      return null;
    }
  } catch {
    return null;
  }

  return {
    name: name.trim(),
    browserDownloadUrl,
    size,
  };
};

export const parseLatestRelease = (value: unknown): LatestRelease => {
  if (
    !isRecord(value) ||
    typeof value.tag_name !== "string" ||
    !Array.isArray(value.assets)
  ) {
    throw new Error("The latest release response has an unexpected shape.");
  }

  const tagName = value.tag_name.trim();

  if (tagName === "") {
    throw new Error("The latest release is missing a tag name.");
  }

  return {
    tagName,
    assets: value.assets
      .map((asset) => parseReleaseAsset(asset))
      .filter((asset): asset is ReleaseAsset => asset !== null),
  };
};

const hasProjectName = (name: string): boolean => /vs[._ -]?parallel/i.test(name);
const isX64 = (name: string): boolean =>
  /(?:^|[._-])(?:x64|x86_64|amd64)(?:[._-]|$)/i.test(name);
const isArm = (name: string): boolean =>
  /(?:^|[._-])(?:arm64|aarch64)(?:[._-]|$)/i.test(name);

const selectBestAsset = (
  assets: ReleaseAsset[],
  accepts: (name: string) => boolean,
  score: (name: string) => number,
): ReleaseAsset | undefined =>
  assets
    .filter((asset) => accepts(asset.name))
    .sort(
      (left, right) =>
        score(right.name) - score(left.name) || left.name.localeCompare(right.name),
    )[0];

export const resolveInstallerAssets = (assets: ReleaseAsset[]): InstallerAssets => {
  const macosDmgs = assets.filter(
    (asset) => hasProjectName(asset.name) && /\.dmg$/i.test(asset.name),
  );
  const universalMacosDmg = selectBestAsset(
    macosDmgs,
    (name) => /universal/i.test(name),
    () => 0,
  );
  const macosDmg = universalMacosDmg ?? (macosDmgs.length === 1 ? macosDmgs[0] : undefined);

  const windowsInstaller = selectBestAsset(
    assets,
    (name) =>
      hasProjectName(name) &&
      !isArm(name) &&
      (/(?:setup|installer)[^/]*\.exe$/i.test(name) || /\.msi$/i.test(name)),
    (name) =>
      (isX64(name) ? 5 : 0) +
      (/-setup\.exe$/i.test(name) ? 6 : /installer[^/]*\.exe$/i.test(name) ? 4 : 2),
  );

  const linuxDeb = selectBestAsset(
    assets,
    (name) => hasProjectName(name) && !isArm(name) && /\.deb$/i.test(name),
    (name) => (isX64(name) ? 5 : 0),
  );

  return {
    ...(macosDmg ? { "macos-dmg": macosDmg } : {}),
    ...(windowsInstaller ? { "windows-installer": windowsInstaller } : {}),
    ...(linuxDeb ? { "linux-deb": linuxDeb } : {}),
  };
};

export const detectDesktopPlatform = ({
  userAgent,
  platform,
  clientHintPlatform = "",
  maxTouchPoints,
}: NavigatorSnapshot): DesktopPlatform | null => {
  const identity = `${clientHintPlatform} ${platform} ${userAgent}`.toLowerCase();

  if (/android|cros|iphone|ipad|ipod/.test(identity)) {
    return null;
  }

  if (/mac/.test(`${clientHintPlatform} ${platform}`.toLowerCase()) && maxTouchPoints > 1) {
    return null;
  }

  if (/win/.test(identity)) {
    return "windows";
  }

  if (/mac/.test(identity)) {
    return "macos";
  }

  if (/linux|x11/.test(identity)) {
    return "linux";
  }

  return null;
};

const getNavigatorSnapshot = (navigatorValue: Navigator): NavigatorSnapshot => {
  const navigatorWithClientHints = navigatorValue as Navigator & {
    userAgentData?: { platform?: string };
  };

  return {
    userAgent: navigatorValue.userAgent,
    platform: navigatorValue.platform,
    clientHintPlatform: navigatorWithClientHints.userAgentData?.platform,
    maxTouchPoints: navigatorValue.maxTouchPoints,
  };
};

const formatFileSize = (bytes: number): string => {
  const megabytes = bytes / 1_000_000;
  return `${megabytes >= 10 ? megabytes.toFixed(0) : megabytes.toFixed(1)} MB`;
};

const fetchLatestRelease = async (): Promise<LatestRelease> => {
  const controller = new AbortController();
  const timeout = window.setTimeout(() => controller.abort(), 8_000);

  try {
    const response = await fetch(LATEST_RELEASE_API_URL, {
      headers: {
        Accept: "application/vnd.github+json",
      },
      credentials: "omit",
      referrerPolicy: "no-referrer",
      signal: controller.signal,
    });

    if (!response.ok) {
      throw new Error(`The latest release request returned ${response.status}.`);
    }

    return parseLatestRelease(await response.json());
  } finally {
    window.clearTimeout(timeout);
  }
};

const highlightPlatform = (platform: DesktopPlatform): void => {
  const card = document.querySelector<HTMLElement>(`[data-platform-card="${platform}"]`);

  if (!card) {
    return;
  }

  card.classList.add("is-recommended");
  const recommendation = card.querySelector<HTMLElement>("[data-platform-recommendation]");

  if (recommendation) {
    recommendation.hidden = false;
  }
};

const updateDownloadLink = (
  link: HTMLAnchorElement,
  kind: InstallerKind,
  asset: ReleaseAsset,
): void => {
  const spec = DOWNLOAD_SPECS[kind];
  const label = link.querySelector<HTMLElement>("[data-download-label]");
  const size = formatFileSize(asset.size);

  link.href = asset.browserDownloadUrl;
  link.dataset.downloadResolved = "true";
  link.setAttribute("aria-label", `${spec.ariaLabel}: ${asset.name}, ${size}`);
  link.title = `${asset.name} (${size})`;

  if (label) {
    label.textContent = spec.label;
  }
};

const emphasizeRecommendedDownload = (
  platform: DesktopPlatform,
  installerAssets: InstallerAssets,
  links: HTMLAnchorElement[],
): void => {
  const preferredKind = PREFERRED_DOWNLOADS[platform].find((kind) => installerAssets[kind]);

  if (!preferredKind) {
    return;
  }

  const preferredLink = links.find((link) => link.dataset.downloadKind === preferredKind);
  preferredLink?.classList.add("is-recommended");
};

export const initializeDownloads = (): void => {
  const links = Array.from(
    document.querySelectorAll<HTMLAnchorElement>("[data-download-kind]"),
  );

  if (links.length === 0) {
    return;
  }

  const status = document.querySelector<HTMLElement>("[data-release-status]");
  const platform = detectDesktopPlatform(getNavigatorSnapshot(window.navigator));

  if (platform) {
    highlightPlatform(platform);
  }

  if (status) {
    status.hidden = false;
    status.textContent = "Finding the latest installers…";
  }

  void fetchLatestRelease()
    .then((release) => {
      const installerAssets = resolveInstallerAssets(release.assets);
      let resolvedCount = 0;

      for (const link of links) {
        const kind = link.dataset.downloadKind;

        if (!isInstallerKind(kind)) {
          continue;
        }

        const asset = installerAssets[kind];

        if (asset) {
          updateDownloadLink(link, kind, asset);
          resolvedCount += 1;
        }
      }

      if (platform) {
        emphasizeRecommendedDownload(platform, installerAssets, links);
      }

      if (!status) {
        return;
      }

      if (resolvedCount === links.length) {
        status.textContent = `${release.tagName} installers are ready.`;
      } else if (resolvedCount > 0) {
        status.textContent =
          `${release.tagName}: ${resolvedCount} of ${links.length} downloads are ready. ` +
          "Missing formats open the latest release.";
      } else {
        status.textContent =
          "No direct installers were found. Platform links open the latest release.";
      }
    })
    .catch(() => {
      if (status) {
        status.textContent =
          "Direct downloads are unavailable right now. Platform links open the latest release.";
      }
    });
};
