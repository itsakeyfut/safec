int g(int a);
int g();
int f(int *p) {
    return g(p);
}
int h(void) {
    return g();
}
