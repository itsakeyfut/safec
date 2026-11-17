void *malloc(int n);
void *memset(void *s, int c, int n);
int keep(void *p);
int release_all(void);

int main(void) {
    int r;
    int *a = malloc(4);
    if (a == 0) {
        return 0;
    }
    r = keep(memset(a, 0, 4)) + release_all();
    return r;
}
