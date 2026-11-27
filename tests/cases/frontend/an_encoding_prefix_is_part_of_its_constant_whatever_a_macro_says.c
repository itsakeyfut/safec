#define L
int prefixed[L'\xff' == 255 ? 1 : -1];
int main(void) {
    return L'\xff' == 255;
}
