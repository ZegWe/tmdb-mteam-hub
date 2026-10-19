const IMG_BASE = "https://image.tmdb.org/t/p/w342";

export function posterUrl(path) {
  return path ? `${IMG_BASE}${path}` : "";
}

export function itemImageUrl(item) {
  return item?.poster_url || item?.cover_url || posterUrl(item?.poster_path) || "";
}

// Search images are browser requests, including older Douban proxy URLs.
export function browserImageUrl(url) {
  if (!url?.startsWith("/api/douban/image?")) return url || "";
  try {
    const upstream = new URL(url, "http://localhost").searchParams.get("url");
    const parsed = new URL(upstream);
    return parsed.protocol === "https:" ? upstream : "";
  } catch {
    return "";
  }
}

export function searchImageUrl(item) {
  return browserImageUrl(itemImageUrl(item));
}
