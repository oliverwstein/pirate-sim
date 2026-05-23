import os
import argparse

def generate_repo_doc(repo_path, output_file, allowed_extensions):
    # Normalize extensions (ensure they start with a dot)
    valid_exts = tuple(ext if ext.startswith('.') else f'.{ext}' for ext in allowed_extensions)

    def build_tree(startpath):
        tree = []
        for root, dirs, files in os.walk(startpath):
            # Ignore hidden directories (e.g., .git, .pytest_cache)
            dirs[:] = [d for d in dirs if not d.startswith('.')]
            
            level = root.replace(startpath, '').count(os.sep)
            indent = '│   ' * (level - 1) + '├── ' if level > 0 else ''
            
            # Add directory name to tree
            if level > 0:
                tree.append(f"{indent}{os.path.basename(root)}/")
            
            sub_indent = '│   ' * level
            for f in files:
                # Filter by allowed extensions if any are specified
                if not valid_exts or f.endswith(valid_exts):
                    tree.append(f"{sub_indent}├── {f}")
        return "\n".join(tree)

    def extract_contents(startpath):
        contents = []
        for root, dirs, files in os.walk(startpath):
            dirs[:] = [d for d in dirs if not d.startswith('.')]
            for f in files:
                if not valid_exts or f.endswith(valid_exts):
                    file_path = os.path.join(root, f)
                    rel_path = os.path.relpath(file_path, startpath)
                    
                    try:
                        with open(file_path, 'r', encoding='utf-8') as file:
                            content = file.read()
                            
                        # Determine file extension for syntax highlighting
                        ext = os.path.splitext(f)[1].replace('.', '')
                        
                        contents.append(f"## {rel_path}\n")
                        contents.append(f"```{ext}\n{content}\n```\n\n")
                    except Exception as e:
                        contents.append(f"## {rel_path}\n")
                        contents.append(f"_Could not read file or it contains non-text characters: {e}_\n\n")
        return "\n".join(contents)

    # Generate the Markdown strings
    print("Building file tree...")
    file_tree = build_tree(repo_path)
    
    print("Extracting file contents...")
    file_contents = extract_contents(repo_path)

    # Write to output file
    print(f"Writing to {output_file}...")
    with open(output_file, 'w', encoding='utf-8') as out:
        out.write("# Repository Structure\n\n")
        out.write("```text\n" + file_tree + "\n```\n\n")
        out.write("# File Contents\n\n" + file_contents)

    print("Done! 🎉")

if __name__ == "__main__":
    parser = argparse.ArgumentParser(description="Generate a Markdown document of a repository's structure and contents.")
    parser.add_argument("repo_path", help="Path to the local repository directory")
    parser.add_argument("-o", "--output", default="repo_dump.md", help="Output Markdown file name (default: repo_dump.md)")
    parser.add_argument("-t", "--types", nargs="*", default=[], help="List of file extensions to include (e.g., py md js). Leave empty to include all.")

    args = parser.parse_args()
    generate_repo_doc(args.repo_path, args.output, args.types)
