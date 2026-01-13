//! Viewer module - generates HTML pages for viewing slides with OpenSeadragon.

use crate::server::handlers::SlideMetadataResponse;

/// Escape HTML special characters to prevent XSS attacks.
fn html_escape(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => result.push_str("&amp;"),
            '<' => result.push_str("&lt;"),
            '>' => result.push_str("&gt;"),
            '"' => result.push_str("&quot;"),
            '\'' => result.push_str("&#x27;"),
            _ => result.push(c),
        }
    }
    result
}

/// Generate an HTML page with OpenSeadragon viewer for a slide.
///
/// # Arguments
///
/// * `slide_id` - The slide identifier (will be URL-encoded in tile URLs)
/// * `metadata` - Slide metadata containing dimensions and level info
/// * `base_url` - Base URL for tile requests (e.g., "http://localhost:3000")
/// * `auth_query` - Optional query string for authentication (e.g., "&exp=...&sig=...")
pub fn generate_viewer_html(
    slide_id: &str,
    metadata: &SlideMetadataResponse,
    base_url: &str,
    auth_query: &str,
) -> String {
    let base_url = base_url.trim_end_matches('/');
    let encoded_slide_id = urlencoding::encode(slide_id);

    // Get tile size from level 0 (or default to 256)
    let tile_size = metadata.levels.first().map(|l| l.tile_width).unwrap_or(256);

    // Calculate max level for OpenSeadragon (OSD uses inverted levels)
    let max_level = metadata.level_count.saturating_sub(1);

    // Build level dimensions JSON for the tile source
    let level_dimensions: Vec<String> = metadata
        .levels
        .iter()
        .map(|l| format!("{{ width: {}, height: {} }}", l.width, l.height))
        .collect();

    // Escape user-controlled values to prevent XSS
    let escaped_slide_id = html_escape(slide_id);
    let escaped_format = html_escape(&metadata.format);

    format!(
        r##"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>WSI Viewer - {escaped_slide_id}</title>
    <script src="https://cdn.jsdelivr.net/npm/openseadragon@4.1/build/openseadragon.min.js"></script>
    <style>
        * {{
            margin: 0;
            padding: 0;
            box-sizing: border-box;
        }}
        body {{
            background: #0f0f0f;
            font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, Oxygen, Ubuntu, sans-serif;
            overflow: hidden;
        }}
        #viewer {{
            width: 100vw;
            height: 100vh;
        }}
        .info-panel {{
            position: absolute;
            top: 16px;
            left: 16px;
            background: rgba(0, 0, 0, 0.85);
            color: #fff;
            padding: 16px 20px;
            border-radius: 8px;
            font-size: 13px;
            line-height: 1.5;
            backdrop-filter: blur(10px);
            border: 1px solid rgba(255, 255, 255, 0.1);
            max-width: 320px;
            z-index: 1000;
        }}
        .info-panel h2 {{
            font-size: 14px;
            font-weight: 600;
            margin-bottom: 8px;
            color: #fff;
            word-break: break-all;
        }}
        .info-panel .meta {{
            color: rgba(255, 255, 255, 0.7);
            font-size: 12px;
        }}
        .info-panel .meta span {{
            color: rgba(255, 255, 255, 0.9);
        }}
        .info-panel .format-badge {{
            display: inline-block;
            background: rgba(99, 102, 241, 0.2);
            color: #818cf8;
            padding: 2px 8px;
            border-radius: 4px;
            font-size: 11px;
            font-weight: 500;
            margin-top: 8px;
        }}
        .controls-hint {{
            position: absolute;
            bottom: 16px;
            left: 16px;
            background: rgba(0, 0, 0, 0.7);
            color: rgba(255, 255, 255, 0.6);
            padding: 8px 12px;
            border-radius: 6px;
            font-size: 11px;
            backdrop-filter: blur(10px);
        }}
        .controls-hint kbd {{
            background: rgba(255, 255, 255, 0.15);
            padding: 2px 6px;
            border-radius: 3px;
            margin: 0 2px;
        }}
        .loading {{
            position: absolute;
            top: 50%;
            left: 50%;
            transform: translate(-50%, -50%);
            color: rgba(255, 255, 255, 0.5);
            font-size: 14px;
        }}
        .error-banner {{
            position: absolute;
            top: 0;
            left: 0;
            right: 0;
            background: rgba(220, 38, 38, 0.95);
            color: white;
            padding: 12px 20px;
            font-size: 14px;
            z-index: 1000;
            display: none;
            backdrop-filter: blur(10px);
        }}
        .error-banner.visible {{
            display: block;
        }}
        .error-banner strong {{
            font-weight: 600;
        }}
        .error-banner .error-details {{
            font-size: 12px;
            opacity: 0.9;
            margin-top: 4px;
        }}
    </style>
</head>
<body>
    <div id="error-banner" class="error-banner">
        <strong>Failed to load tiles</strong>
        <div class="error-details" id="error-details"></div>
    </div>

    <div id="viewer">
        <div class="loading">Loading slide...</div>
    </div>

    <div class="info-panel">
        <h2>{escaped_slide_id}</h2>
        <div class="meta">
            <span>{width}</span> x <span>{height}</span> px<br>
            <span>{level_count}</span> pyramid levels<br>
            Tile size: <span>{tile_size}</span> px
        </div>
        <div class="format-badge">{escaped_format}</div>
    </div>

    <div class="controls-hint">
        <kbd>+</kbd>/<kbd>-</kbd> Zoom &nbsp; <kbd>Home</kbd> Reset &nbsp; <kbd>F</kbd> Fullscreen
    </div>

    <script>
        // Level dimensions from server metadata
        const levelDimensions = [{level_dimensions}];
        const levelCount = {level_count};
        const maxLevel = {max_level};

        // Create custom tile source
        const tileSource = {{
            height: {height},
            width: {width},
            tileSize: {tile_size},
            minLevel: 0,
            maxLevel: maxLevel,

            getLevelScale: function(level) {{
                // OpenSeadragon level 0 is lowest resolution, but we want level 0 to be highest
                // So we need to invert: OSD level N maps to our level (maxLevel - N)
                const ourLevel = maxLevel - level;
                if (ourLevel < 0 || ourLevel >= levelCount) return 0;
                return levelDimensions[ourLevel].width / {width};
            }},

            getNumTiles: function(level) {{
                const ourLevel = maxLevel - level;
                if (ourLevel < 0 || ourLevel >= levelCount) return {{ x: 0, y: 0 }};
                const dims = levelDimensions[ourLevel];
                return {{
                    x: Math.ceil(dims.width / {tile_size}),
                    y: Math.ceil(dims.height / {tile_size})
                }};
            }},

            getTileUrl: function(level, x, y) {{
                // Map OSD level to our pyramid level (inverted)
                const ourLevel = maxLevel - level;
                return "{base_url}/tiles/{encoded_slide_id}/" + ourLevel + "/" + x + "/" + y + ".jpg{auth_query}";
            }}
        }};

        // Initialize OpenSeadragon
        const viewer = OpenSeadragon({{
            id: "viewer",
            prefixUrl: "https://cdn.jsdelivr.net/npm/openseadragon@4.1/build/openseadragon/images/",
            tileSources: tileSource,
            showNavigator: true,
            navigatorPosition: "BOTTOM_RIGHT",
            navigatorSizeRatio: 0.15,
            showRotationControl: true,
            showFullPageControl: true,
            showZoomControl: true,
            showHomeControl: true,
            gestureSettingsMouse: {{
                clickToZoom: true,
                dblClickToZoom: true,
                scrollToZoom: true
            }},
            gestureSettingsTouch: {{
                pinchToZoom: true
            }},
            animationTime: 0.3,
            blendTime: 0.1,
            maxZoomPixelRatio: 2,
            visibilityRatio: 0.5,
            constrainDuringPan: true,
            immediateRender: false,
            crossOriginPolicy: "Anonymous"
        }});

        // Track errors
        let errorCount = 0;
        let firstErrorShown = false;

        // Remove loading message when first tile loads
        viewer.addHandler('tile-loaded', function() {{
            const loading = document.querySelector('.loading');
            if (loading) loading.remove();
        }});

        // Handle tile load errors
        viewer.addHandler('tile-load-failed', function(event) {{
            errorCount++;

            // Show error banner on first failure
            if (!firstErrorShown) {{
                firstErrorShown = true;
                const banner = document.getElementById('error-banner');
                const details = document.getElementById('error-details');

                // Try to determine the error type
                let errorMessage = 'Unable to load slide tiles. ';
                if (event.message) {{
                    if (event.message.includes('401')) {{
                        errorMessage += 'Authentication failed - the viewer token may have expired. Try refreshing the page.';
                    }} else if (event.message.includes('404')) {{
                        errorMessage += 'Tile not found - the slide may have been moved or deleted.';
                    }} else if (event.message.includes('415')) {{
                        errorMessage += 'Unsupported slide format.';
                    }} else {{
                        errorMessage += event.message;
                    }}
                }} else {{
                    errorMessage += 'Check your network connection and try refreshing the page.';
                }}

                details.textContent = errorMessage;
                banner.classList.add('visible');

                // Also update loading message
                const loading = document.querySelector('.loading');
                if (loading) {{
                    loading.textContent = 'Error loading slide';
                    loading.style.color = 'rgba(220, 38, 38, 0.8)';
                }}
            }}
        }});

        // Keyboard shortcuts
        document.addEventListener('keydown', function(e) {{
            if (e.key === 'f' || e.key === 'F') {{
                if (viewer.isFullPage()) {{
                    viewer.setFullPage(false);
                }} else {{
                    viewer.setFullPage(true);
                }}
            }}
        }});
    </script>
</body>
</html>"##,
        escaped_slide_id = escaped_slide_id,
        escaped_format = escaped_format,
        width = metadata.width,
        height = metadata.height,
        level_count = metadata.level_count,
        tile_size = tile_size,
        level_dimensions = level_dimensions.join(", "),
        max_level = max_level,
        base_url = base_url,
        encoded_slide_id = encoded_slide_id,
        auth_query = auth_query,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::handlers::LevelMetadataResponse;

    fn test_metadata() -> SlideMetadataResponse {
        SlideMetadataResponse {
            slide_id: "test.svs".to_string(),
            format: "Aperio SVS".to_string(),
            width: 50000,
            height: 40000,
            level_count: 3,
            levels: vec![
                LevelMetadataResponse {
                    level: 0,
                    width: 50000,
                    height: 40000,
                    tile_width: 256,
                    tile_height: 256,
                    tiles_x: 196,
                    tiles_y: 157,
                    downsample: 1.0,
                },
                LevelMetadataResponse {
                    level: 1,
                    width: 12500,
                    height: 10000,
                    tile_width: 256,
                    tile_height: 256,
                    tiles_x: 49,
                    tiles_y: 40,
                    downsample: 4.0,
                },
                LevelMetadataResponse {
                    level: 2,
                    width: 3125,
                    height: 2500,
                    tile_width: 256,
                    tile_height: 256,
                    tiles_x: 13,
                    tiles_y: 10,
                    downsample: 16.0,
                },
            ],
        }
    }

    #[test]
    fn test_generate_viewer_html_contains_slide_info() {
        let metadata = test_metadata();
        let html = generate_viewer_html("test.svs", &metadata, "http://localhost:3000", "");

        assert!(html.contains("test.svs"));
        assert!(html.contains("50000"));
        assert!(html.contains("40000"));
        // Level count is wrapped in span: <span>3</span> pyramid levels
        assert!(html.contains(">3</span> pyramid levels"));
        assert!(html.contains("Aperio SVS"));
    }

    #[test]
    fn test_generate_viewer_html_contains_openseadragon() {
        let metadata = test_metadata();
        let html = generate_viewer_html("test.svs", &metadata, "http://localhost:3000", "");

        assert!(html.contains("openseadragon"));
        assert!(html.contains("OpenSeadragon"));
    }

    #[test]
    fn test_generate_viewer_html_contains_tile_url() {
        let metadata = test_metadata();
        let html = generate_viewer_html("test.svs", &metadata, "http://localhost:3000", "");

        assert!(html.contains("/tiles/test.svs/"));
        assert!(html.contains(".jpg"));
    }

    #[test]
    fn test_generate_viewer_html_with_auth_query() {
        let metadata = test_metadata();
        let html = generate_viewer_html(
            "test.svs",
            &metadata,
            "http://localhost:3000",
            "?exp=123&sig=abc",
        );

        assert!(html.contains("?exp=123&sig=abc"));
    }

    #[test]
    fn test_generate_viewer_html_encodes_slide_id() {
        let metadata = test_metadata();
        let html = generate_viewer_html(
            "folder/sub folder/test.svs",
            &metadata,
            "http://localhost:3000",
            "",
        );

        // Should URL-encode the slide_id in tile URLs
        assert!(html.contains("folder%2Fsub%20folder%2Ftest.svs"));
    }

    #[test]
    fn test_generate_viewer_html_contains_level_dimensions() {
        let metadata = test_metadata();
        let html = generate_viewer_html("test.svs", &metadata, "http://localhost:3000", "");

        // Should contain level dimension objects
        assert!(html.contains("width: 50000, height: 40000"));
        assert!(html.contains("width: 12500, height: 10000"));
        assert!(html.contains("width: 3125, height: 2500"));
    }

    #[test]
    fn test_html_escape_basic() {
        assert_eq!(html_escape("hello"), "hello");
        assert_eq!(html_escape(""), "");
        assert_eq!(html_escape("test.svs"), "test.svs");
    }

    #[test]
    fn test_html_escape_special_chars() {
        assert_eq!(html_escape("<script>"), "&lt;script&gt;");
        assert_eq!(html_escape("a & b"), "a &amp; b");
        assert_eq!(html_escape("\"quoted\""), "&quot;quoted&quot;");
        assert_eq!(html_escape("it's"), "it&#x27;s");
        assert_eq!(
            html_escape("<script>alert('xss')</script>"),
            "&lt;script&gt;alert(&#x27;xss&#x27;)&lt;/script&gt;"
        );
    }

    #[test]
    fn test_generate_viewer_html_escapes_xss_in_slide_id() {
        let mut metadata = test_metadata();
        metadata.slide_id = "<script>alert(1)</script>".to_string();

        let html = generate_viewer_html(
            "<script>alert(1)</script>",
            &metadata,
            "http://localhost:3000",
            "",
        );

        // The literal script tag should NOT appear unescaped
        assert!(!html.contains("<script>alert(1)</script>"));
        // The escaped version should appear
        assert!(html.contains("&lt;script&gt;alert(1)&lt;/script&gt;"));
    }

    #[test]
    fn test_generate_viewer_html_escapes_xss_in_format() {
        let mut metadata = test_metadata();
        metadata.format = "<img onerror=alert(1)>".to_string();

        let html = generate_viewer_html("test.svs", &metadata, "http://localhost:3000", "");

        // The literal img tag should NOT appear unescaped
        assert!(!html.contains("<img onerror=alert(1)>"));
        // The escaped version should appear in the format badge
        assert!(html.contains("&lt;img onerror=alert(1)&gt;"));
    }
}
