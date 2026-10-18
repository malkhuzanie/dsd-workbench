//! A small in-place radix-2 FFT.

use std::f32::consts::PI;

/// Forward transform. Does nothing unless both slices have the same power-of-two length.
pub fn fft(re: &mut [f32], im: &mut [f32]) {
    let n = re.len();
    if n < 2 || !n.is_power_of_two() || im.len() != n {
        return;
    }
    let bits = n.trailing_zeros();
    for i in 0..n {
        let j = i.reverse_bits() >> (usize::BITS - bits);
        if j > i {
            re.swap(i, j);
            im.swap(i, j);
        }
    }

    let mut len = 2;
    while len <= n {
        let angle = -2.0 * PI / len as f32;
        let half = len / 2;
        for start in (0..n).step_by(len) {
            for k in 0..half {
                let (s, c) = (angle * k as f32).sin_cos();
                let (a, b) = (start + k, start + k + half);
                let tr = re[b] * c - im[b] * s;
                let ti = re[b] * s + im[b] * c;
                re[b] = re[a] - tr;
                im[b] = im[a] - ti;
                re[a] += tr;
                im[a] += ti;
            }
        }
        len <<= 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sine_peaks_in_its_bin() {
        let n = 256;
        let mut re: Vec<f32> = (0..n)
            .map(|i| (2.0 * PI * 16.0 * i as f32 / n as f32).sin())
            .collect();
        let mut im = vec![0.0; n];
        fft(&mut re, &mut im);
        let peak = (0..n / 2)
            .max_by(|a, b| re[*a].hypot(im[*a]).total_cmp(&re[*b].hypot(im[*b])))
            .unwrap_or(0);
        assert_eq!(peak, 16);
    }
}
