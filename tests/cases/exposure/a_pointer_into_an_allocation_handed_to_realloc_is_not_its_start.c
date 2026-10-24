void *malloc(int n);
void free(void *p);
void *realloc(void *p, int n);

int main(void) {
    int *p = malloc(8);
    if (p == 0) {
        return 0;
    }
    int *q = realloc(p + 1, 16);
    free(q);
    return 0;
}
