void *malloc(int n);
void *realloc(void *p, int n);
void free(void *p);

int main(int c) {
    int *p = malloc(4);
    if (p == 0) {
        return 0;
    }
    p[0] = 1;
    int *q = realloc(p, 8);
    if (c) {
        int *r = malloc(4);
        q = r;
    }
    if (q == 0) {
        free(p);
        return 0;
    }
    return 0;
}
