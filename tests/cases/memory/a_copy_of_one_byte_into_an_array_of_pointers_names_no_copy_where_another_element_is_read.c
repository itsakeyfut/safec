// A copy of one byte reaches `arr[0]` alone, so a read of `arr[1]` names no
// copy (#918's review).
void *memcpy(void *d, void *s, int n);
int main(void) {
    int x = 0;
    int *arr[2];
    arr[0] = &x;
    arr[1] = &x;
    char b[16];
    b[0] = 0;
    void *v = arr;
    memcpy(v, b, 1);
    int *p = arr[1];
    return p == 0;
}
