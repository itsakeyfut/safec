void h(void) {
}

int g(int a) {
    return a;
}

int f(void) {
    return g(h());
}
