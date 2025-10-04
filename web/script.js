// Raven Website JavaScript
document.addEventListener('DOMContentLoaded', function() {
    // Mobile Navigation Toggle
    const navToggle = document.querySelector('.nav-toggle');
    const navLinks = document.querySelector('.nav-links');
    
    if (navToggle && navLinks) {
        navToggle.addEventListener('click', function() {
            navLinks.classList.toggle('active');
            navToggle.classList.toggle('active');
            
            // Prevent body scroll when menu is open
            if (navLinks.classList.contains('active')) {
                document.body.style.overflow = 'hidden';
            } else {
                document.body.style.overflow = '';
            }
        });
        
        // Close mobile menu when clicking on a link
        const mobileNavLinks = navLinks.querySelectorAll('.nav-link');
        mobileNavLinks.forEach(link => {
            link.addEventListener('click', function() {
                navLinks.classList.remove('active');
                navToggle.classList.remove('active');
                document.body.style.overflow = '';
            });
        });
        
        // Close mobile menu when clicking outside
        document.addEventListener('click', function(e) {
            if (!navToggle.contains(e.target) && !navLinks.contains(e.target)) {
                navLinks.classList.remove('active');
                navToggle.classList.remove('active');
                document.body.style.overflow = '';
            }
        });
        
        // Close mobile menu on window resize (if screen becomes larger)
        window.addEventListener('resize', function() {
            if (window.innerWidth > 768) {
                navLinks.classList.remove('active');
                navToggle.classList.remove('active');
                document.body.style.overflow = '';
            }
        });
    }

    // Smooth Scrolling for Navigation Links
    const navLinksAll = document.querySelectorAll('.nav-link[href^="#"]');
    navLinksAll.forEach(link => {
        link.addEventListener('click', function(e) {
            e.preventDefault();
            const targetId = this.getAttribute('href');
            const targetSection = document.querySelector(targetId);
            
            if (targetSection) {
                const offsetTop = targetSection.offsetTop - 80; // Account for fixed navbar
                window.scrollTo({
                    top: offsetTop,
                    behavior: 'smooth'
                });
            }
        });
    });

    // Examples Tab Functionality
    const tabButtons = document.querySelectorAll('.tab-btn');
    const tabContents = document.querySelectorAll('.tab-content');
    
    tabButtons.forEach(button => {
        button.addEventListener('click', function() {
            const targetTab = this.getAttribute('data-tab');
            
            // Remove active class from all buttons and contents
            tabButtons.forEach(btn => btn.classList.remove('active'));
            tabContents.forEach(content => content.classList.remove('active'));
            
            // Add active class to clicked button and corresponding content
            this.classList.add('active');
            const targetContent = document.getElementById(targetTab);
            if (targetContent) {
                targetContent.classList.add('active');
            }
        });
    });

    // Navbar Background on Scroll (optimized version below)
    const navbar = document.querySelector('.navbar');

    // Animate Elements on Scroll
    const observerOptions = {
        threshold: 0.1,
        rootMargin: '0px 0px -50px 0px'
    };

    const observer = new IntersectionObserver(function(entries) {
        entries.forEach(entry => {
            if (entry.isIntersecting) {
                entry.target.style.opacity = '1';
                entry.target.style.transform = 'translateY(0)';
            }
        });
    }, observerOptions);

    // Observe feature cards, download cards, and doc cards
    const animatedElements = document.querySelectorAll('.feature-card, .download-card, .doc-card');
    animatedElements.forEach(el => {
        el.style.opacity = '0';
        el.style.transform = 'translateY(20px)';
        el.style.transition = 'opacity 0.6s ease, transform 0.6s ease';
        observer.observe(el);
    });

    // Copy Code Functionality
    const codeBlocks = document.querySelectorAll('.code-example, .install-code');
    codeBlocks.forEach(block => {
        const pre = block.querySelector('pre');
        if (pre) {
            pre.style.position = 'relative';
            
            // Create copy button
            const copyButton = document.createElement('button');
            copyButton.innerHTML = '📋';
            copyButton.style.position = 'absolute';
            copyButton.style.top = '10px';
            copyButton.style.right = '10px';
            copyButton.style.background = 'rgba(255, 255, 255, 0.1)';
            copyButton.style.border = 'none';
            copyButton.style.borderRadius = '4px';
            copyButton.style.padding = '5px 8px';
            copyButton.style.cursor = 'pointer';
            copyButton.style.color = 'white';
            copyButton.style.fontSize = '12px';
            copyButton.style.opacity = '0.7';
            copyButton.style.transition = 'opacity 0.3s ease';
            
            copyButton.addEventListener('mouseenter', function() {
                this.style.opacity = '1';
            });
            
            copyButton.addEventListener('mouseleave', function() {
                this.style.opacity = '0.7';
            });
            
            copyButton.addEventListener('click', function() {
                const code = pre.textContent;
                navigator.clipboard.writeText(code).then(function() {
                    copyButton.innerHTML = '✅';
                    setTimeout(function() {
                        copyButton.innerHTML = '📋';
                    }, 2000);
                }).catch(function(err) {
                    console.error('Failed to copy code: ', err);
                });
            });
            
            pre.appendChild(copyButton);
        }
    });

    // Download Button Functionality
    const downloadButtons = document.querySelectorAll('.btn[href="#"]');
    downloadButtons.forEach(button => {
        button.addEventListener('click', function(e) {
            e.preventDefault();
            
            // Show coming soon message
            const originalText = this.textContent;
            this.textContent = 'Coming Soon!';
            this.style.background = '#10b981';
            
            setTimeout(() => {
                this.textContent = originalText;
                this.style.background = '';
            }, 2000);
        });
    });

    // Typing Animation for Hero Title
    const titleMain = document.querySelector('.title-main');
    if (titleMain) {
        const text = titleMain.textContent;
        titleMain.textContent = '';
        
        let i = 0;
        const typeWriter = function() {
            if (i < text.length) {
                titleMain.textContent += text.charAt(i);
                i++;
                setTimeout(typeWriter, 100);
            }
        };
        
        // Start typing animation after a short delay
        setTimeout(typeWriter, 500);
    }

    // Parallax Effect for Hero Section
    const hero = document.querySelector('.hero');
    if (hero) {
        window.addEventListener('scroll', function() {
            const scrolled = window.pageYOffset;
            const rate = scrolled * -0.5;
            hero.style.transform = `translateY(${rate}px)`;
        });
    }

    // Stats Counter Animation
    const stats = document.querySelectorAll('.stat-number');
    const statsObserver = new IntersectionObserver(function(entries) {
        entries.forEach(entry => {
            if (entry.isIntersecting) {
                const target = entry.target;
                const finalValue = target.textContent;
                
                if (finalValue === 'v1.1.0') {
                    // Don't animate version number
                    return;
                }
                
                if (finalValue === '100%') {
                    animateCounter(target, 0, 100, '%');
                } else if (finalValue === 'Fast') {
                    // Don't animate text
                    return;
                }
            }
        });
    }, { threshold: 0.5 });
    
    stats.forEach(stat => statsObserver.observe(stat));
    
    function animateCounter(element, start, end, suffix = '') {
        const duration = 2000;
        const startTime = performance.now();
        
        function updateCounter(currentTime) {
            const elapsed = currentTime - startTime;
            const progress = Math.min(elapsed / duration, 1);
            
            const current = Math.floor(start + (end - start) * progress);
            element.textContent = current + suffix;
            
            if (progress < 1) {
                requestAnimationFrame(updateCounter);
            }
        }
        
        requestAnimationFrame(updateCounter);
    }

    // Add hover effects to cards
    const cards = document.querySelectorAll('.feature-card, .download-card, .doc-card');
    cards.forEach(card => {
        card.addEventListener('mouseenter', function() {
            this.style.transform = 'translateY(-8px) scale(1.02)';
        });
        
        card.addEventListener('mouseleave', function() {
            this.style.transform = 'translateY(0) scale(1)';
        });
    });

    // Mobile-specific optimizations
    function isMobile() {
        return window.innerWidth <= 768;
    }
    
    // Optimize animations for mobile
    if (isMobile()) {
        // Reduce animation complexity on mobile
        const animatedElements = document.querySelectorAll('.feature-card, .download-card, .doc-card');
        animatedElements.forEach(el => {
            el.style.transition = 'opacity 0.3s ease, transform 0.3s ease';
        });
        
        // Disable parallax on mobile for better performance
        if (hero) {
            hero.style.transform = 'none';
        }
    }
    
    // Touch-friendly interactions
    const touchElements = document.querySelectorAll('.btn, .tab-btn, .nav-link');
    touchElements.forEach(element => {
        element.addEventListener('touchstart', function() {
            this.style.transform = 'scale(0.98)';
        });
        
        element.addEventListener('touchend', function() {
            this.style.transform = 'scale(1)';
        });
    });
    
    // Prevent zoom on double tap for iOS
    let lastTouchEnd = 0;
    document.addEventListener('touchend', function(event) {
        const now = (new Date()).getTime();
        if (now - lastTouchEnd <= 300) {
            event.preventDefault();
        }
        lastTouchEnd = now;
    }, false);
    
    // Optimize scroll performance on mobile
    let ticking = false;
    function updateScrollEffects() {
        // Navbar background update
        if (navbar) {
            if (window.scrollY > 50) {
                navbar.style.background = 'rgba(255, 255, 255, 0.98)';
                navbar.style.boxShadow = '0 2px 20px rgba(0, 0, 0, 0.1)';
            } else {
                navbar.style.background = 'rgba(255, 255, 255, 0.95)';
                navbar.style.boxShadow = 'none';
            }
        }
        
        ticking = false;
    }
    
    window.addEventListener('scroll', function() {
        if (!ticking) {
            requestAnimationFrame(updateScrollEffects);
            ticking = true;
        }
    });

    // Console Easter Egg
    console.log('%c🦅 Welcome to Raven!', 'color: #00d4ff; font-size: 20px; font-weight: bold;');
    console.log('%cRaven Programming Language v1.1.0', 'color: #1a1a2e; font-size: 14px;');
    console.log('%cBuilt with Rust • Fast • Safe • Expressive', 'color: #64748b; font-size: 12px;');
    console.log('%cGitHub: https://github.com/martian56/raven', 'color: #00d4ff; font-size: 12px;');
});
