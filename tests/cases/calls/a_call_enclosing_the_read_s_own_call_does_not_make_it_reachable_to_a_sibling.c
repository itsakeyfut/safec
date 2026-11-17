void *malloc(int n);
void *memcpy(void *d, void *s, int n);
void **slot(void);
int keep(void *p);
int release_all(void);

int main(void) {
    void **out_ = slot();
    int r;
    int *a = malloc(4);
    if (a == 0) {
        return 0;
    }
    r = keep(memcpy(out_, &a, 8)) + release_all();
    return r;
}
