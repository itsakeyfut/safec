void *malloc(int n);
void *memset(void *s, int c, int n);
void *memcpy(void *d, void *s, int n);
int now(void);
int count(int *p);
int weight(int v);
int **slot_of(int key);
void watch(int **pp);
int emit(int a, int b, int c);
int f(int **out) {
    int *a;
    int *b;
    int *cursor;
    int x;
    int r;
    cursor = 0;
    watch(&cursor);
    a = malloc(4);
    if (a == 0) {
        return 0;
    }
    b = malloc(4);
    if (b == 0) {
        return 0;
    }
    a[0] = 1;
    b[0] = 2;
    r = (x = a[0]) + (cursor = a + now(), 0);
    return r;
}
