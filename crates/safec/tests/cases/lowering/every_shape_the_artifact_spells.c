void nothing(void) {
}

int every(int a, int b, char c, int **pp) {
    int x;
    int *p;

    x = a * b;
    x = a / b;
    x = a % b;
    x = a + b;
    x = a - b;
    x = a << b;
    x = a >> b;
    x = a < b;
    x = a > b;
    x = a <= b;
    x = a >= b;
    x = a == b;
    x = a != b;
    x = a & b;
    x = a ^ b;
    x = a | b;
    x = -a;
    x = !a;
    x = ~a;
    x = +a;
    p = &x;
    **pp = x;
    c = c;
    return **pp;
}
