void *malloc(int n);
void *memmove(void *d, void *s, int n);
void *memset(void *d, int c, int n);

int main(void) {
    char *p = malloc(8);
    char *q = malloc(4);
    if (p == 0) {
        return 1;
    }
    if (q == 0) {
        return 1;
    }
    memset(q, 0, 4);
    memmove(p, q, 8);
    return 0;
}
