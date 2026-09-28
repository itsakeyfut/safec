void *malloc(int n);
void *memset(void *s, int c, int n);
int keep(int *p);
int release_all(void);

int main(void) {
    int r;
    int *a = malloc(4);
    int *b = malloc(4);
    if (a == 0) {
        return 0;
    }
    if (b == 0) {
        return 0;
    }
    r = keep(a) + (memset(b, 0, 4) != 0) + release_all();
    return r;
}
