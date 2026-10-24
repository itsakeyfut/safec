void *malloc(int n);
void free(void *p);

int *maybe(int *p, int c) {
    if (c) {
        free(p);
    }
    return p;
}

int main(void) {
    int *a = malloc(4);
    if (a == 0) {
        return 0;
    }
    int *q = maybe(a, 1);
    if (q == 0) {
        return 0;
    }
    return *q;
}
