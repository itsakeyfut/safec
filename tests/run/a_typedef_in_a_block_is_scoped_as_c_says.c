typedef char T;
int main(void) {
    int r = 0;
    {
        typedef long T;
        T z = 0;
        r = r + (int)z + (sizeof(T) == sizeof(long) ? 0 : 1);
    }
    {
        int T = sizeof(T);
        r = r + (T == sizeof(int) ? 0 : 1);
    }
    {
        int a = 4, b = sizeof(a);
        r = r + (b == sizeof(int) ? 0 : 1);
    }
    r = r + (sizeof(T) == 1 ? 0 : 1);
    goto T;
T:
    return r;
}
