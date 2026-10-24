void h(void) {
}

int f(void) {
    int x = h();
    char c = h();
    int *p = h();
    return x + c + (p == 0);
}
