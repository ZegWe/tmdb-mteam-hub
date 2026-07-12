export function cardKey(item, fallback) {
  return `${item.source || item.media_type || fallback}-${item.id ?? item.subject_id ?? item.title}`;
}

export function cardSubtitle(item) {
  if ((item.source || item.media_type) === "douban") {
    const ratingValue = item.rating?.value ?? item.vote_average;
    const bits = [
      item.year || item.abstract_2 || item.date || "",
      ratingValue != null ? `★ ${Number(ratingValue).toFixed(1)}` : "",
    ];
    return bits.filter(Boolean).join(" · ");
  }
  const date = item.release_date || item.first_air_date || "";
  const ratingValue = item.vote_average;
  return [date, ratingValue != null ? `★ ${Number(ratingValue).toFixed(1)}` : ""]
    .filter(Boolean)
    .join(" · ");
}
