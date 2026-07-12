const IMG_BASE = "https://image.tmdb.org/t/p/w342";

export function posterUrl(path) {
  return path ? `${IMG_BASE}${path}` : "";
}

export function itemImageUrl(item) {
  return item?.poster_url || item?.cover_url || posterUrl(item?.poster_path) || "";
}
