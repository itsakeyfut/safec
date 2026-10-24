void *malloc(int n);
void free(void *p);

int f(int c) {
    int *p = malloc(4);
    if (p == 0) {
        return 0;
    }
    int **pp = &p;
    if (c) {
        free(p);
    }
    return **pp;
}
