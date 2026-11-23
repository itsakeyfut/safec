struct Fixed {
    int a;
    const int b;
};
int fixed(void) {
    struct Fixed f;
    return 0;
}
struct Huge {
    char a[0x1000000000000000];
    char b[0x1000000000000000];
    char c[0x1000000000000000];
    int x;
};
int huge(void) {
    struct Huge h;
    return 0;
}
int main(void) {
    return 0;
}
