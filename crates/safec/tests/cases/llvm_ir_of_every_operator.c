int ops(int a, int b) {
    int r;
    r = 0;
    r = r + (a + b);
    r = r + (a - b);
    r = r + (a * b);
    r = r + (a / b);
    r = r + (a % b);
    r = r + (a << b);
    r = r + (a >> b);
    r = r + (a < b);
    r = r + (a > b);
    r = r + (a <= b);
    r = r + (a >= b);
    r = r + (a == b);
    r = r + (a != b);
    r = r + (a & b);
    r = r + (a ^ b);
    r = r + (a | b);
    r = r + (-a);
    r = r + (!a);
    r = r + (~a);
    return r;
}

int main() {
    return ops(7, 2);
}
