void exit(int status);
int g(int a, int b);

int f(int *p) {
    return *p + (exit(1), 0);
}

int h(int *q) {
    return g((exit(1), 0), *q);
}
