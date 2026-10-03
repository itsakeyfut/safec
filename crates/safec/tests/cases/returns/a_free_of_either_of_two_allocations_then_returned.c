void *malloc(int n);
void free(void *p);

int *either(int c) {
    int *a = malloc(4);
    int *b = malloc(4);
    int *p = a;
    if (c) {
        p = b;
    }
    free(p);
    return p;
}
