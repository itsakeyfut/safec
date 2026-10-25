void *malloc(int n);
int memcmp(void *a, void *b, int n);

int main(void) {
    char *p = malloc(4);
    char *q = malloc(4);
    if (p == 0) {
        return 1;
    }
    if (q == 0) {
        return 1;
    }
    *p = 0;
    *q = 0;
    return memcmp(p, q, 100);
}
