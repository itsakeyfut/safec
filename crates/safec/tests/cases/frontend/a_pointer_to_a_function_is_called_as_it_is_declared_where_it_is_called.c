int two(int a, int b) {
    return a + b;
}
int (*fp)() = two;
int main(void) {
    return fp(1, 2) - 3;
}
int (*fp)(int);
