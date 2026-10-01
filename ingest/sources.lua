local function summary_prompt(item)
  return "Summarize this source item without adding facts. Preserve names and version numbers. " ..
    "Return two short sentences and cite the canonical URL.\n\nTitle: " .. item.title ..
    "\nURL: " .. item.canonical_url .. "\nSource summary: " .. (item.summary or "")
end

return {
  sources = {
    {
      id = "hacker-news-top", display_name = "Hacker News Top", enabled = true, limit = 20,
      transport = { kind = "hacker_news", list = "top" },
      rights = { mode = "facts", attribution = "Hacker News", commercial_use = true },
      transform = "news_item",
    },
    {
      id = "uv-releases", display_name = "uv Releases", enabled = true, limit = 10,
      transport = { kind = "github_releases", owner = "astral-sh", repo = "uv" },
      rights = { mode = "facts", attribution = "GitHub Releases", commercial_use = true },
      transform = "release",
    },
    {
      id = "cloudflare-blog", display_name = "Cloudflare Blog", enabled = true, limit = 10,
      transport = { kind = "rss", url = "https://blog.cloudflare.com/rss/" },
      rights = { mode = "index", attribution = "Cloudflare Blog", commercial_use = true },
      transform = "news_item",
    },
    {
      id = "bbc-pidgin", display_name = "BBC News Pidgin", enabled = false, limit = 10,
      transport = { kind = "rss", url = "https://feeds.bbci.co.uk/pidgin/rss.xml" },
      rights = { mode = "requires_license", attribution = "BBC News Pidgin", commercial_use = false },
      transform = "news_item",
    },
  },
  transforms = {
    news_item = function(item, rights)
      return {
        headline = item.title, attribution = rights.attribution, source_url = item.canonical_url,
        carousel = { format = "headline_source_v1", slides = { item.title, rights.attribution, item.canonical_url } },
        llm = { system = "You transform attributed source metadata into concise Tardy copy.", prompt = summary_prompt(item), max_output_tokens = 180 },
      }
    end,
    release = function(item, rights)
      return {
        headline = item.title, attribution = rights.attribution, source_url = item.canonical_url,
        carousel = { format = "release_notes_v1", slides = { item.title, item.facts.tag or "release", item.canonical_url } },
        llm = { system = "You summarize software releases from supplied release notes only.", prompt = summary_prompt(item), max_output_tokens = 220 },
      }
    end,
  },
}
