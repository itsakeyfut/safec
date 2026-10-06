void h(void) {
}

int f(void) {
    int x = h();
    int *p = h();
    return x + (p == 0);
}
