int g(int a);
int h(int *q);
int used(int *p) {
    if (!p) {
        return 0;
    }
    int x = g(*p) + h(p);
    return x;
}
int *returned(int *p) {
    h(p);
    return p;
}
