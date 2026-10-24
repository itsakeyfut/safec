void *malloc(int n);
void free(void *p);
void *memcpy(void *d, void *s, int n);

int main(void) {
    int *a = malloc(4);
    int *b = malloc(4);
    if (a == 0) {
        return 0;
    }
    if (b == 0) {
        return 0;
    }
    *a = 1;
    memcpy(b, a, 4);
    free(a);
    free(b);
    return 0;
}
