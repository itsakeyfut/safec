void free(void *p);
int g(int a);
int h(int *q);

int call_first(int *p) {
    if (!p) {
        return 0;
    }
    int x = g(*p) + h(p) + (free(p), 0);
    return x;
}

int free_first(int *p) {
    if (!p) {
        return 0;
    }
    int x = g(*p) + (free(p), 0) + h(p);
    return x;
}
