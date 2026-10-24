int g(int *p);

int f(int x) {
    return (*g)(x);
}
