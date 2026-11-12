// A string copy stops at the null and pads with zeros (C17 7.24.2.4 p2, p3),
// so `arr[1]` is not named as bytes it copied (#918's review).
char *strncpy(char *d, char *s, int n);
int main(void) {
    int x = 0;
    int *arr[2];
    arr[0] = &x;
    arr[1] = &x;
    int *src[2];
    src[0] = 0;
    void *v = arr;
    void *s = src;
    strncpy(v, s, 16);
    int *p = arr[1];
    return p == 0;
}
