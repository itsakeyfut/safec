extern int g[10];
int g[2];
int h[2][3];
int h[2][4];
int (*p)[2];
int (*p)[3];
extern int k[];
int k[4];
int m[2];
int m[1 + 1];
extern int a[];
int a[2];
int a[3];
void f(int (*q)[2]);
void f(int (*q)[3]);
void e(int q[2][3]);
void e(int q[2][4]);
void d(int q[2][3]);
void d(int (*q)[4]);
int n[2];
int main(void) {
    extern int n[3];
    return 0;
}
