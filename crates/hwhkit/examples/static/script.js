// HwhKit 示例应用 JavaScript

// 页面加载完成后执行
document.addEventListener('DOMContentLoaded', function() {
    console.log('🚀 HwhKit 示例应用已加载');
    
    // 添加页面加载动画
    addPageAnimations();
    
    // 检查 API 状态
    checkApiHealth();
    
    // 初始化事件监听器
    initEventListeners();
});

// 添加页面动画
function addPageAnimations() {
    // 为卡片添加进入动画
    const cards = document.querySelectorAll('.feature-card, .user-card');
    cards.forEach((card, index) => {
        card.style.opacity = '0';
        card.style.transform = 'translateY(30px)';
        
        setTimeout(() => {
            card.style.transition = 'all 0.6s ease';
            card.style.opacity = '1';
            card.style.transform = 'translateY(0)';
        }, index * 100);
    });
}

// 检查 API 健康状态
async function checkApiHealth() {
    try {
        const response = await fetch('/api/v1/health');
        if (response.ok) {
            const data = await response.json();
            console.log('✅ API 健康状态:', data);
            
            // 更新页面上的状态指示器
            updateHealthStatus(true);
        } else {
            console.warn('⚠️ API 健康检查失败');
            updateHealthStatus(false);
        }
    } catch (error) {
        console.error('❌ API 健康检查错误:', error);
        updateHealthStatus(false);
    }
}

// 更新健康状态显示
function updateHealthStatus(isHealthy) {
    const statusElements = document.querySelectorAll('.health-status');
    statusElements.forEach(element => {
        element.textContent = isHealthy ? '🟢 在线' : '🔴 离线';
        element.className = `health-status ${isHealthy ? 'online' : 'offline'}`;
    });
}

// 初始化事件监听器
function initEventListeners() {
    // 点击外部关闭模态框
    window.addEventListener('click', function(event) {
        const modal = document.getElementById('addUserModal');
        if (event.target === modal) {
            hideAddUserForm();
        }
    });
    
    // ESC 键关闭模态框
    document.addEventListener('keydown', function(event) {
        if (event.key === 'Escape') {
            hideAddUserForm();
        }
    });
    
    // 表单验证
    const forms = document.querySelectorAll('form');
    forms.forEach(form => {
        form.addEventListener('submit', function(event) {
            if (!validateForm(form)) {
                event.preventDefault();
            }
        });
    });
}

// 表单验证
function validateForm(form) {
    const requiredFields = form.querySelectorAll('[required]');
    let isValid = true;
    
    requiredFields.forEach(field => {
        if (!field.value.trim()) {
            showFieldError(field, '此字段为必填项');
            isValid = false;
        } else {
            clearFieldError(field);
        }
        
        // 邮箱格式验证
        if (field.type === 'email' && field.value.trim()) {
            const emailRegex = /^[^\s@]+@[^\s@]+\.[^\s@]+$/;
            if (!emailRegex.test(field.value)) {
                showFieldError(field, '请输入有效的邮箱地址');
                isValid = false;
            }
        }
    });
    
    return isValid;
}

// 显示字段错误
function showFieldError(field, message) {
    clearFieldError(field);
    
    field.style.borderColor = '#dc3545';
    
    const errorDiv = document.createElement('div');
    errorDiv.className = 'field-error';
    errorDiv.style.color = '#dc3545';
    errorDiv.style.fontSize = '0.85rem';
    errorDiv.style.marginTop = '0.25rem';
    errorDiv.textContent = message;
    
    field.parentNode.appendChild(errorDiv);
}

// 清除字段错误
function clearFieldError(field) {
    field.style.borderColor = '#eee';
    
    const existingError = field.parentNode.querySelector('.field-error');
    if (existingError) {
        existingError.remove();
    }
}

// 显示通知消息
function showNotification(message, type = 'info') {
    const notification = document.createElement('div');
    notification.className = `notification notification-${type}`;
    notification.style.cssText = `
        position: fixed;
        top: 20px;
        right: 20px;
        padding: 1rem 1.5rem;
        border-radius: 8px;
        color: white;
        font-weight: 500;
        z-index: 1001;
        transform: translateX(100%);
        transition: transform 0.3s ease;
    `;
    
    // 设置背景颜色
    switch (type) {
        case 'success':
            notification.style.backgroundColor = '#28a745';
            break;
        case 'error':
            notification.style.backgroundColor = '#dc3545';
            break;
        case 'warning':
            notification.style.backgroundColor = '#ffc107';
            notification.style.color = '#333';
            break;
        default:
            notification.style.backgroundColor = '#667eea';
    }
    
    notification.textContent = message;
    document.body.appendChild(notification);
    
    // 显示动画
    setTimeout(() => {
        notification.style.transform = 'translateX(0)';
    }, 100);
    
    // 自动隐藏
    setTimeout(() => {
        notification.style.transform = 'translateX(100%)';
        setTimeout(() => {
            if (notification.parentNode) {
                notification.parentNode.removeChild(notification);
            }
        }, 300);
    }, 3000);
}

// 格式化时间
function formatTime(timestamp) {
    return new Date(timestamp).toLocaleString('zh-CN');
}

// 复制到剪贴板
async function copyToClipboard(text) {
    try {
        await navigator.clipboard.writeText(text);
        showNotification('已复制到剪贴板', 'success');
    } catch (err) {
        console.error('复制失败:', err);
        showNotification('复制失败', 'error');
    }
}

// 防抖函数
function debounce(func, wait) {
    let timeout;
    return function executedFunction(...args) {
        const later = () => {
            clearTimeout(timeout);
            func(...args);
        };
        clearTimeout(timeout);
        timeout = setTimeout(later, wait);
    };
}

// 节流函数
function throttle(func, limit) {
    let inThrottle;
    return function() {
        const args = arguments;
        const context = this;
        if (!inThrottle) {
            func.apply(context, args);
            inThrottle = true;
            setTimeout(() => inThrottle = false, limit);
        }
    };
}

// 全局错误处理
window.addEventListener('error', function(event) {
    console.error('全局错误:', event.error);
    showNotification('发生了一个错误，请刷新页面重试', 'error');
});

// API 请求包装器
class ApiClient {
    constructor(baseURL = '/api/v1') {
        this.baseURL = baseURL;
    }
    
    async request(endpoint, options = {}) {
        const url = `${this.baseURL}${endpoint}`;
        const config = {
            headers: {
                'Content-Type': 'application/json',
                ...options.headers,
            },
            ...options,
        };
        
        try {
            const response = await fetch(url, config);
            
            if (!response.ok) {
                throw new Error(`HTTP error! status: ${response.status}`);
            }
            
            return await response.json();
        } catch (error) {
            console.error('API 请求失败:', error);
            throw error;
        }
    }
    
    async get(endpoint) {
        return this.request(endpoint);
    }
    
    async post(endpoint, data) {
        return this.request(endpoint, {
            method: 'POST',
            body: JSON.stringify(data),
        });
    }
    
    async put(endpoint, data) {
        return this.request(endpoint, {
            method: 'PUT',
            body: JSON.stringify(data),
        });
    }
    
    async delete(endpoint) {
        return this.request(endpoint, {
            method: 'DELETE',
        });
    }
}

// 创建全局 API 客户端实例
window.api = new ApiClient();