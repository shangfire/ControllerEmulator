print("hello lua")

function Callback()
    if rust_callback then
        rust_callback()
    else
        print("rust_callback is nil")
    end
end

function Add(a, b)
    return a + b
end

-- 调用函数并打印结果
local result = Add(10, 20)
Callback()
print("10 + 20 =", result)