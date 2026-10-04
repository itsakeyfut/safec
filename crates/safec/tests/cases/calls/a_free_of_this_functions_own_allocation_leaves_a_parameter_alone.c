void *malloc(int n);
void free(void *p);

int g(int *a) {
    if (a == 0) {
        return 0;
    }
    int *t = malloc(4);
    if (t == 0) {
        return 0;
    }
    t[0] = 1;
    free(t);
    return *a;
}
