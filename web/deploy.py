#!/usr/bin/env python3
import os
import sys
from pathlib import Path

from ufazien import UfazienAPIClient
from ufazien.utils import create_zip, find_website_config

def main():
    print("🚀 Raven - Deployment Script")
    print("=" * 50)
    
    project_dir = Path(__file__).parent.absolute()
    print(f"Project directory: {project_dir}\n")
    
    client = UfazienAPIClient()
    
    if not client.access_token:
        email = os.getenv('UFAZIEN_HOSTING_EMAIL')
        password = os.getenv('UFAZIEN_HOSTING_PASSWORD')
        
        if email and password:
            print("🔐 Logging in to Ufazien using credentials...")
            try:
                user = client.login(email, password)
                print(f"✓ Login successful: {user.get('email', email)}")
            except Exception as e:
                print(f"❌ Error: Login failed: {e}")
                sys.exit(1)
        else:
            print("❌ Error: Not logged in to Ufazien")
            print("Please run: ufazien login")
            print("Or set UFAZIEN_HOSTING_EMAIL and UFAZIEN_HOSTING_PASSWORD environment variables")
            sys.exit(1)
    
    config = find_website_config(str(project_dir))
    
    if not config:
        print("❌ Error: .ufazien.json not found in project directory")
        print("Please run: ufazien create")
        sys.exit(1)
    
    website_id = config.get('website_id')
    if not website_id:
        print("❌ Error: website_id not found in .ufazien.json")
        sys.exit(1)
    
    website_name = config.get('website_name', 'Unknown')
    domain = config.get('domain', '')
    
    print(f"Website: {website_name}")
    print(f"Website ID: {website_id}")
    if domain:
        print(f"Domain: {domain}")
    print()
    
    print("📦 Creating ZIP archive...")
    try:
        zip_path = create_zip(str(project_dir))
        zip_size = os.path.getsize(zip_path) / (1024 * 1024)
        print(f"✓ ZIP archive created: {Path(zip_path).name} ({zip_size:.2f} MB)")
    except Exception as e:
        print(f"❌ Error creating ZIP file: {e}")
        sys.exit(1)
    
    print("\n📤 Uploading files to Ufazien...")
    try:
        response = client.upload_zip(website_id, zip_path)
        print("✓ Files uploaded successfully")
    except Exception as e:
        print(f"❌ Error uploading files: {e}")
        try:
            os.remove(zip_path)
        except:
            pass
        sys.exit(1)
    
    try:
        os.remove(zip_path)
        print("✓ Temporary ZIP file removed")
    except:
        pass
    
    print("\n🚀 Triggering deployment...")
    try:
        deployment = client.deploy_website(website_id)
        status = deployment.get('status', 'queued')
        print(f"✓ Deployment triggered successfully")
        print(f"  Status: {status}")
    except Exception as e:
        print(f"⚠ Warning: Could not trigger deployment: {e}")
        print("Files have been uploaded. Deployment may start automatically.")
    
    print("\n" + "=" * 50)
    print("✅ Deployment process completed!")
    if domain:
        print(f"🌐 Your website: https://{domain}")
    print("\nNote: It may take a few minutes for the deployment to complete.")
    print("Check your Ufazien dashboard for deployment status.")

if __name__ == "__main__":
    main()