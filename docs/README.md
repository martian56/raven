# Raven Documentation

This directory contains the complete documentation for the Raven programming language, built with MkDocs and Material theme.

## 🚀 Quick Start

### Local Development

```bash
# Install dependencies
pip install mkdocs mkdocs-material

# Start development server
mkdocs serve

# Open http://127.0.0.1:8000 in your browser
```

### Building Documentation

```bash
# Build static site
mkdocs build

# Files will be in the 'site' directory
```

### Deploying to GitHub Pages

```bash
# Deploy to GitHub Pages (requires git setup)
mkdocs gh-deploy

# Or use the GitHub Action (automatic on push to main)
```

## 📁 Structure

```
docs/
├── index.md                    # Homepage
├── syntax.md                   # Language syntax reference
├── PRODUCTION_ROADMAP.md       # Development roadmap
├── STDLIB_SPEC.md             # Standard library specification
├── getting-started/            # Installation and quick start
│   ├── installation.md
│   ├── quick-start.md
│   └── rvpm-and-format.md      # rv.toml, rvpm fmt, [fmt]
├── language-reference/         # Language features
│   └── data-types.md
├── standard-library/           # Standard library docs
│   └── overview.md
├── examples/                   # Code examples
│   └── basic.md
├── development/                # Contributing and development
│   └── contributing.md
└── resources/                  # Tools and resources
    └── vscode-extension.md
```

## 🎨 Features

- **Material Design** - Clean, modern interface
- **Dark/Light Mode** - Automatic theme switching
- **Search** - Full-text search across all documentation
- **Mobile Responsive** - Works on all devices
- **Code Highlighting** - Syntax highlighting for Raven code
- **Navigation** - Easy navigation with tabs and sections

## 📝 Writing Documentation

### Adding New Pages

1. **Create markdown file** in appropriate directory
2. **Add to navigation** in `mkdocs.yml`
3. **Use proper headings** (H1 for page title, H2+ for sections)
4. **Include code examples** with proper syntax highlighting

### Code Blocks

Use triple backticks with language specification:

````markdown
```raven
fun main() -> void {
    print("Hello, Raven!");
}
```
````

### Links

- **Internal links**: `[Text](../path/file.md)`
- **External links**: `[Text](https://example.com)`
- **Anchor links**: `[Text](../file.md#section)`

## 🔧 Configuration

The `mkdocs.yml` file contains:

- **Site metadata** - Name, description, URL
- **Theme settings** - Material theme configuration
- **Navigation** - Page structure and order
- **Plugins** - Search and other features
- **Extensions** - Markdown extensions for enhanced features

## 🚀 Deployment

### GitHub Pages (Automatic)

The documentation is automatically deployed to GitHub Pages when you push to the `main` branch using the GitHub Action in `.github/workflows/docs.yml`.

**URL**: https://martian56.github.io/raven

### Manual Deployment

```bash
# Build and deploy
mkdocs gh-deploy

# Or build locally and upload manually
mkdocs build
# Upload 'site' directory contents to your web server
```

## 🛠️ Customization

### Theme Colors

Edit the `palette` section in `mkdocs.yml`:

```yaml
theme:
  palette:
    - media: "(prefers-color-scheme: light)"
      scheme: default
      primary: indigo  # Change this color
      accent: indigo
```

### Adding Features

- **Search**: Already enabled
- **Comments**: Can be added with plugins
- **Analytics**: Add Google Analytics or similar
- **Custom CSS**: Add to `docs/stylesheets/extra.css`

## 📚 Content Guidelines

### Writing Style

- **Clear and concise** - Get to the point quickly
- **Code examples** - Show, don't just tell
- **Progressive complexity** - Start simple, build up
- **Consistent formatting** - Follow the established patterns

### Code Examples

- **Test all examples** - Make sure they work
- **Use meaningful names** - `name` not `x`
- **Add comments** - Explain complex logic
- **Show output** - Include expected results

## 🔍 SEO and Metadata

The documentation includes:

- **Meta descriptions** - For search engines
- **Open Graph tags** - For social media sharing
- **Structured data** - For better search results
- **Sitemap** - Automatic generation

## 📱 Mobile Support

The documentation is fully responsive and works great on:

- **Desktop** - Full navigation and features
- **Tablet** - Optimized layout
- **Mobile** - Touch-friendly interface

## 🤝 Contributing

To contribute to the documentation:

1. **Fork the repository**
2. **Create a branch** for your changes
3. **Make your changes** following the guidelines
4. **Test locally** with `mkdocs serve`
5. **Submit a pull request**

## 📞 Support

If you need help with the documentation:

- **GitHub Issues** - Report problems or request features
- **Discussions** - Ask questions or share ideas
- **Pull Requests** - Contribute improvements

---

**Happy documenting!** 📚✨
